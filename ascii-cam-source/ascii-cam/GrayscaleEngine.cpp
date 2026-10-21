#include "pch.h"
#include "GrayscaleEngine.h"
#include <d3dcompiler.h>

#pragma comment(lib, "d3dcompiler.lib")

static const char* g_GrayscaleComputeShaderHLSL = R"(
Texture2D<float> inputTex : register(t0);
RWTexture2D<float4> outputTex : register(u0);

[numthreads(8, 8, 1)]
void main(uint3 dtid : SV_DispatchThreadID)
{
    uint width, height;
    inputTex.GetDimensions(width, height);

    uint2 pos = dtid.xy;
    if (pos.x >= width || pos.y >= height)
        return;

    // 입력은 이미 휘도(Y)이므로 그대로 사용
    float luminance = inputTex.Load(int3(pos, 0));

    // 흑백으로 출력 (R=G=B)
    outputTex[pos] = float4(luminance, luminance, luminance, 1.0f);
}
)";

GrayscaleEngine::~GrayscaleEngine()
{
    Shutdown();
}

HRESULT GrayscaleEngine::Initialize(UINT width, UINT height)
{
    _width = width;
    _height = height;

    UINT createDeviceFlags = 0;

#ifdef _DEBUG
    createDeviceFlags |= D3D11_CREATE_DEVICE_DEBUG;
#endif

    D3D_FEATURE_LEVEL featureLevels[] =
    {
        D3D_FEATURE_LEVEL_11_0
    };

    D3D_FEATURE_LEVEL outFeatureLevel;

    RETURN_IF_FAILED(
        D3D11CreateDevice(
            nullptr,
            D3D_DRIVER_TYPE_HARDWARE,
            nullptr,
            createDeviceFlags,
            featureLevels,
            _countof(featureLevels),
            D3D11_SDK_VERSION,
            &_device,
            &outFeatureLevel,
            &_context
        )
    );

    wil::com_ptr_nothrow<ID3DBlob> shaderBlob;
    wil::com_ptr_nothrow<ID3DBlob> errorBlob;

    HRESULT hr = D3DCompile(
        g_GrayscaleComputeShaderHLSL,
        strlen(g_GrayscaleComputeShaderHLSL),
        "GrayscaleCS",
        nullptr,
        nullptr,
        "main",
        "cs_5_0",
        0,
        0,
        &shaderBlob,
        &errorBlob
    );

    if (FAILED(hr))
    {
        if (errorBlob)
        {
            OutputDebugStringA(
                static_cast<const char*>(errorBlob->GetBufferPointer())
            );
        }

        return hr;
    }

    RETURN_IF_FAILED(
        _device->CreateComputeShader(
            shaderBlob->GetBufferPointer(),
            shaderBlob->GetBufferSize(),
            nullptr,
            &_computeShader
        )
    );

    D3D11_TEXTURE2D_DESC inputDesc = {};
    inputDesc.Width = _width;
    inputDesc.Height = _height;
    inputDesc.MipLevels = 1;
    inputDesc.ArraySize = 1;
    inputDesc.Format = DXGI_FORMAT_R8_UNORM;
    inputDesc.SampleDesc.Count = 1;
    inputDesc.Usage = D3D11_USAGE_DYNAMIC;
    inputDesc.CPUAccessFlags = D3D11_CPU_ACCESS_WRITE;
    inputDesc.BindFlags = D3D11_BIND_SHADER_RESOURCE;

    RETURN_IF_FAILED(
        _device->CreateTexture2D(
            &inputDesc,
            nullptr,
            &_inputTexture
        )
    );

    RETURN_IF_FAILED(
        _device->CreateShaderResourceView(
            _inputTexture.get(),
            nullptr,
            &_inputSRV
        )
    );

    D3D11_TEXTURE2D_DESC outputDesc = {};
    outputDesc.Width = _width;
    outputDesc.Height = _height;
    outputDesc.MipLevels = 1;
    outputDesc.ArraySize = 1;
    outputDesc.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
    outputDesc.SampleDesc.Count = 1;
    outputDesc.Usage = D3D11_USAGE_DEFAULT;
    outputDesc.BindFlags = D3D11_BIND_UNORDERED_ACCESS;

    RETURN_IF_FAILED(
        _device->CreateTexture2D(
            &outputDesc,
            nullptr,
            &_outputTexture
        )
    );

    RETURN_IF_FAILED(
        _device->CreateUnorderedAccessView(
            _outputTexture.get(),
            nullptr,
            &_outputUAV
        )
    );

    D3D11_TEXTURE2D_DESC stagingDesc = {};
    stagingDesc.Width = _width;
    stagingDesc.Height = _height;
    stagingDesc.MipLevels = 1;
    stagingDesc.ArraySize = 1;
    stagingDesc.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
    stagingDesc.SampleDesc.Count = 1;
    stagingDesc.Usage = D3D11_USAGE_STAGING;
    stagingDesc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;

    RETURN_IF_FAILED(
        _device->CreateTexture2D(
            &stagingDesc,
            nullptr,
            &_stagingTexture
        )
    );

    return S_OK;
}


HRESULT GrayscaleEngine::Process(
    const BYTE * yPlane,
    LONG pitch
)
{
    if (!_inputTexture ||
        !_inputSRV ||
        !_outputTexture ||
        !_outputUAV ||
        !_stagingTexture ||
        !_context ||
        !_computeShader ||
        !yPlane)
    {
        return E_FAIL;
    }

    D3D11_MAPPED_SUBRESOURCE mappedInput = {};

    RETURN_IF_FAILED(
        _context->Map(
            _inputTexture.get(),
            0,
            D3D11_MAP_WRITE_DISCARD,
            0,
            &mappedInput
        )
    );

    const BYTE* srcRow = yPlane;
    BYTE* dstRow = static_cast<BYTE*>(mappedInput.pData);

    for (UINT y = 0; y < _height; ++y)
    {
        memcpy(dstRow, srcRow, _width);
        srcRow += pitch;
        dstRow += mappedInput.RowPitch;
    }

    _context->Unmap(_inputTexture.get(), 0);

    _context->CSSetShader(
        _computeShader.get(),
        nullptr,
        0
    );

    ID3D11ShaderResourceView* srvs[] =
    {
        _inputSRV.get()
    };

    _context->CSSetShaderResources(
        0,
        1,
        srvs
    );

    ID3D11UnorderedAccessView* uavs[] =
    {
        _outputUAV.get()
    };

    _context->CSSetUnorderedAccessViews(
        0,
        1,
        uavs,
        nullptr
    );

    const UINT dispatchX = (_width + 7) / 8;
    const UINT dispatchY = (_height + 7) / 8;

    _context->Dispatch(
        dispatchX,
        dispatchY,
        1
    );

    ID3D11ShaderResourceView* nullSRV = nullptr;
    _context->CSSetShaderResources(
        0,
        1,
        &nullSRV
    );

    ID3D11UnorderedAccessView* nullUAV = nullptr;
    _context->CSSetUnorderedAccessViews(
        0,
        1,
        &nullUAV,
        nullptr
    );

    _context->CopyResource(
        _stagingTexture.get(),
        _outputTexture.get()
    );

    D3D11_MAPPED_SUBRESOURCE mappedStaging = {};

    RETURN_IF_FAILED(
        _context->Map(
            _stagingTexture.get(),
            0,
            D3D11_MAP_READ,
            0,
            &mappedStaging
        )
    );

    const BYTE* srcResultRow =
        static_cast<const BYTE*>(mappedStaging.pData);

    BYTE* dstResultRow = const_cast<BYTE*>(yPlane);

    for (UINT y = 0; y < _height; ++y)
    {
        const BYTE* srcPixel = srcResultRow;
        BYTE* dstPixel = dstResultRow;

        for (UINT x = 0; x < _width; ++x)
        {
            dstPixel[x] = srcPixel[x * 4];
        }

        srcResultRow += mappedStaging.RowPitch;
        dstResultRow += pitch;
    }

    _context->Unmap(
        _stagingTexture.get(),
        0
    );

    // ============================================================
    // NV12 기준: UV 평면을 128로 채워서 완전 흑백으로 만듦
    // ============================================================
    BYTE* uvPlane = const_cast<BYTE*>(yPlane) + (pitch * _height);
    const UINT uvHeight = _height / 2;
    const UINT uvSize = static_cast<UINT>(pitch) * uvHeight;

    memset(uvPlane, 128, uvSize);

    return S_OK;
}

void GrayscaleEngine::Shutdown()
{
    _computeShader.reset();
    _stagingTexture.reset();
    _outputUAV.reset();
    _outputTexture.reset();
    _inputSRV.reset();
    _inputTexture.reset();
    _context.reset();
    _device.reset();
}