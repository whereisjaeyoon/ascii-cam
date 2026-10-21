#include "pch.h"
#include "Filter.h"
#include "Nv12Layout.h"
#include <d3dcompiler.h>
#include <vector>
#include <cstring>
#include <type_traits>
#include <new>
#pragma comment(lib, "d3dcompiler.lib")

namespace
{
    // Works with both legacy void and HRESULT-returning AsciiEngine revisions.
    template<class Function> HRESULT RunAscii(Function&& function)
    {
        if constexpr (std::is_void_v<std::invoke_result_t<Function>>)
        { function(); return S_OK; }
        else { return function(); }
    }
}

// Grayscale pixel processing is performed by a hardware D3D11 compute shader.
// Upload/readback and MF buffer copies remain CPU bridges; this is NOT zero-copy.
struct GpuGrayscale
{
    UINT byteCount = 0;
    wil::com_ptr_nothrow<ID3D11Device> device;
    wil::com_ptr_nothrow<ID3D11DeviceContext> context;
    wil::com_ptr_nothrow<ID3D11Buffer> input, output, staging, parameters;
    wil::com_ptr_nothrow<ID3D11ShaderResourceView> srv;
    wil::com_ptr_nothrow<ID3D11UnorderedAccessView> uav;
    wil::com_ptr_nothrow<ID3D11ComputeShader> shader;

    HRESULT Initialize(UINT width, UINT height)
    {
        Nv12Layout layout;
        RETURN_HR_IF(E_INVALIDARG, !Nv12Layout::TryCreate(width, height, width, layout) || (layout.packedBytes % 4));
        byteCount = layout.packedBytes;
        const D3D_FEATURE_LEVEL levels[] = { D3D_FEATURE_LEVEL_11_0 };
        D3D_FEATURE_LEVEL actual;
        RETURN_IF_FAILED(D3D11CreateDevice(nullptr, D3D_DRIVER_TYPE_HARDWARE, nullptr, 0,
            levels, ARRAYSIZE(levels), D3D11_SDK_VERSION, &device, &actual, &context));
        static constexpr char code[] = R"(
ByteAddressBuffer Input : register(t0);
RWByteAddressBuffer Output : register(u0);
cbuffer Params : register(b0) { uint YBytes; uint TotalBytes; uint2 Padding; };
[numthreads(256, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    uint offset = id.x * 4;
    if (offset >= TotalBytes) return;
    Output.Store(offset, offset < YBytes ? Input.Load(offset) : 0x80808080);
})";
        wil::com_ptr_nothrow<ID3DBlob> compiled, errors;
        const HRESULT compileResult = D3DCompile(code, sizeof(code) - 1, "grayscale", nullptr,
            nullptr, "main", "cs_5_0", D3DCOMPILE_ENABLE_STRICTNESS, 0, &compiled, &errors);
        if (FAILED(compileResult))
        {
            if (errors) OutputDebugStringA(static_cast<const char*>(errors->GetBufferPointer()));
            return compileResult;
        }
        RETURN_IF_FAILED(device->CreateComputeShader(compiled->GetBufferPointer(), compiled->GetBufferSize(), nullptr, &shader));
        D3D11_BUFFER_DESC desc{};
        desc.ByteWidth = byteCount;
        desc.Usage = D3D11_USAGE_DEFAULT;
        desc.BindFlags = D3D11_BIND_SHADER_RESOURCE;
        desc.MiscFlags = D3D11_RESOURCE_MISC_BUFFER_ALLOW_RAW_VIEWS;
        RETURN_IF_FAILED(device->CreateBuffer(&desc, nullptr, &input));
        desc.BindFlags = D3D11_BIND_UNORDERED_ACCESS;
        RETURN_IF_FAILED(device->CreateBuffer(&desc, nullptr, &output));
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.MiscFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
        RETURN_IF_FAILED(device->CreateBuffer(&desc, nullptr, &staging));
        D3D11_SHADER_RESOURCE_VIEW_DESC sr{};
        sr.Format = DXGI_FORMAT_R32_TYPELESS;
        sr.ViewDimension = D3D11_SRV_DIMENSION_BUFFEREX;
        sr.BufferEx.NumElements = byteCount / 4;
        sr.BufferEx.Flags = D3D11_BUFFEREX_SRV_FLAG_RAW;
        RETURN_IF_FAILED(device->CreateShaderResourceView(input.get(), &sr, &srv));
        D3D11_UNORDERED_ACCESS_VIEW_DESC uv{};
        uv.Format = DXGI_FORMAT_R32_TYPELESS;
        uv.ViewDimension = D3D11_UAV_DIMENSION_BUFFER;
        uv.Buffer.NumElements = byteCount / 4;
        uv.Buffer.Flags = D3D11_BUFFER_UAV_FLAG_RAW;
        RETURN_IF_FAILED(device->CreateUnorderedAccessView(output.get(), &uv, &uav));
        const UINT values[4] = { width * height, byteCount, 0, 0 };
        desc = {};
        desc.ByteWidth = sizeof(values);
        desc.Usage = D3D11_USAGE_IMMUTABLE;
        desc.BindFlags = D3D11_BIND_CONSTANT_BUFFER;
        D3D11_SUBRESOURCE_DATA data{};
        data.pSysMem = values;
        return device->CreateBuffer(&desc, &data, &parameters);
    }

    HRESULT Process(BYTE* bytes, DWORD length)
    {
        RETURN_HR_IF(MF_E_BUFFERTOOSMALL, length < byteCount);
        context->UpdateSubresource(input.get(), 0, nullptr, bytes, 0, 0);
        ID3D11ShaderResourceView* inputs[] = { srv.get() };
        ID3D11UnorderedAccessView* outputs[] = { uav.get() };
        ID3D11Buffer* constants[] = { parameters.get() };
        context->CSSetShader(shader.get(), nullptr, 0);
        context->CSSetShaderResources(0, 1, inputs);
        context->CSSetUnorderedAccessViews(0, 1, outputs, nullptr);
        context->CSSetConstantBuffers(0, 1, constants);
        context->Dispatch((byteCount / 4 + 255) / 256, 1, 1);
        ID3D11ShaderResourceView* noInput[] = { nullptr };
        ID3D11UnorderedAccessView* noOutput[] = { nullptr };
        context->CSSetShaderResources(0, 1, noInput);
        context->CSSetUnorderedAccessViews(0, 1, noOutput, nullptr);
        context->CopyResource(staging.get(), output.get());
        D3D11_MAPPED_SUBRESOURCE mapped{};
        RETURN_IF_FAILED(context->Map(staging.get(), 0, D3D11_MAP_READ, 0, &mapped));
        std::memcpy(bytes, mapped.pData, byteCount);
        context->Unmap(staging.get(), 0);
        return device->GetDeviceRemovedReason();
    }
};

Filter::Filter() = default;
Filter::~Filter() { Shutdown(); }

HRESULT Filter::Initialize(UINT width, UINT height, FilterOption option, IMFMediaType* inputType)
{
    Shutdown();
    RETURN_HR_IF_NULL(E_POINTER, inputType);
    RETURN_IF_FAILED(inputType->GetGUID(MF_MT_SUBTYPE, &_inputFormat));
    RETURN_IF_FAILED(MFGetAttributeSize(inputType, MF_MT_FRAME_SIZE, &_width, &_height));
    RETURN_HR_IF(MF_E_INVALIDMEDIATYPE, _inputFormat != MFVideoFormat_NV12 ||
        _width != width || _height != height || !width || !height || (width % 2) || (height % 2));
    _stride = MFGetAttributeUINT32(inputType, MF_MT_DEFAULT_STRIDE, width);
    RETURN_HR_IF(MF_E_INVALIDMEDIATYPE, _stride < width || _stride > MAXLONG);
    _option = option;
    if (option == FilterOption::Grayscale)
    {
        _grayscale.reset(new (std::nothrow) GpuGrayscale());
        RETURN_HR_IF(E_OUTOFMEMORY, !_grayscale);
        return _grayscale->Initialize(width, height);
    }
    if (option == FilterOption::ASCII)
        return RunAscii([&] { return _ascii.Initialize(width, height); });
    return option == FilterOption::Passthrough ? S_OK : E_NOTIMPL;
}

HRESULT Filter::Process(IMFSample* sample)
{
    RETURN_HR_IF_NULL(E_POINTER, sample);
    if (_option == FilterOption::Passthrough) return S_OK;
    wil::com_ptr_nothrow<IMFMediaBuffer> buffer;
    RETURN_IF_FAILED(sample->ConvertToContiguousBuffer(&buffer));
    wil::com_ptr_nothrow<IMF2DBuffer> image;
    if (SUCCEEDED(buffer->QueryInterface(IID_PPV_ARGS(&image))))
    {
        if (_option == FilterOption::ASCII)
        {
            BYTE* pixels = nullptr;
            LONG pitch = 0;
            RETURN_IF_FAILED(image->Lock2D(&pixels, &pitch));
            const HRESULT hr = pitch < static_cast<LONG>(_width) ? MF_E_INVALIDMEDIATYPE :
                RunAscii([&] { return _ascii.Process(pixels, pitch); });
            const HRESULT unlock = image->Unlock2D();
            return FAILED(hr) ? hr : unlock;
        }
        DWORD length = 0;
        RETURN_IF_FAILED(image->GetContiguousLength(&length));
        try
        {
            std::vector<BYTE> packed(length);
            RETURN_IF_FAILED(image->ContiguousCopyTo(packed.data(), length));
            RETURN_IF_FAILED(_grayscale->Process(packed.data(), length));
            return image->ContiguousCopyFrom(packed.data(), length);
        }
        catch (const std::bad_alloc&) { return E_OUTOFMEMORY; }
    }
    BYTE* data = nullptr;
    DWORD length = 0;
    RETURN_IF_FAILED(buffer->Lock(&data, nullptr, &length));
    HRESULT hr = S_OK;
    Nv12Layout layout;
    if (!Nv12Layout::TryCreate(_width, _height, _stride, layout) || layout.footprint > length)
        hr = MF_E_BUFFERTOOSMALL;
    else if (_option == FilterOption::ASCII)
        hr = RunAscii([&] { return _ascii.Process(data, static_cast<LONG>(_stride)); });
    else if (_stride == _width) hr = _grayscale->Process(data, length);
    else
    {
        // Only layout copies are done on CPU; UV processing stays in the shader.
        try
        {
            std::vector<BYTE> packed(static_cast<size_t>(_width) * _height * 3 / 2);
            hr = MFCopyImage(packed.data(), _width, data, _stride, _width, _height * 3 / 2);
            if (SUCCEEDED(hr)) hr = _grayscale->Process(packed.data(), static_cast<DWORD>(packed.size()));
            if (SUCCEEDED(hr)) hr = MFCopyImage(data, _stride, packed.data(), _width, _width, _height * 3 / 2);
        }
        catch (const std::bad_alloc&) { hr = E_OUTOFMEMORY; }
    }
    const HRESULT unlock = buffer->Unlock();
    return FAILED(hr) ? hr : unlock;
}

void Filter::Shutdown()
{
    _grayscale.reset();
    _ascii.Shutdown();
    _inputFormat = GUID_NULL;
    _option = FilterOption::Passthrough;
}
