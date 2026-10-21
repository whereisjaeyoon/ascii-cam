#include "pch.h"
#include "Filter.h"

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
    Shutdown();
}

HRESULT Filter::Initialize(UINT width, UINT height, FilterOption option)
{
    _width = width;
    _height = height;

    // Engines are sized for the previous resolution; recreate them on demand.
    _asciiReady = false;
    _grayscaleReady = false;

    RETURN_IF_FAILED(EnsureEngine(option));
    _option = option;

    return S_OK;
}

HRESULT Filter::SetOption(FilterOption option)
{
    if (option == _option)
        return S_OK;

    RETURN_IF_FAILED(EnsureEngine(option));
    _option = option;

    return S_OK;
}

HRESULT Filter::EnsureEngine(FilterOption option)
{
    if (option == FilterOption::Grayscale && !_grayscaleReady)
    {
        RETURN_IF_FAILED(_grayscale.Initialize(_width, _height));
        _grayscaleReady = true;
    }

    if (option == FilterOption::ASCII && !_asciiReady)
    {
        RETURN_IF_FAILED(_ascii.Initialize(_width, _height));
        _asciiReady = true;
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
    RETURN_IF_FAILED(sample->GetBufferByIndex(0, &buffer));

    wil::com_ptr_nothrow<IMF2DBuffer> buffer2D;

    if (SUCCEEDED(buffer->QueryInterface(IID_PPV_ARGS(&buffer2D))))
    {
        BYTE* data = nullptr;
        LONG pitch = 0;

        RETURN_IF_FAILED(buffer2D->Lock2D(&data, &pitch));

        HRESULT hr = S_OK;

        if (_option == FilterOption::ASCII)
        {
            hr = _ascii.Process(data, pitch);
        }
        else if (_option == FilterOption::Grayscale)
        {
            hr = _grayscale.Process(data, pitch);
        }

        HRESULT unlockHr = buffer2D->Unlock2D();

        RETURN_IF_FAILED(hr);
        RETURN_IF_FAILED(unlockHr);

        return S_OK;
    }
    BYTE* data = nullptr;
    DWORD maxLength = 0;
    DWORD currentLength = 0;

    RETURN_IF_FAILED(buffer->Lock(&data, &maxLength, &currentLength));

    HRESULT hr = S_OK;

    if (_option == FilterOption::ASCII)
    {
        hr = _ascii.Process(data, static_cast<LONG>(_width));
    }
    else if (_option == FilterOption::Grayscale)
    {
        hr = _grayscale.Process(data, static_cast<LONG>(_width));
    }

    HRESULT unlockHr = buffer->Unlock();

    RETURN_IF_FAILED(hr);
    RETURN_IF_FAILED(unlockHr);

    return S_OK;
}

void Filter::Shutdown()
{
    _ascii.Shutdown();
    _grayscale.Shutdown();
    _asciiReady = false;
    _grayscaleReady = false;
}
