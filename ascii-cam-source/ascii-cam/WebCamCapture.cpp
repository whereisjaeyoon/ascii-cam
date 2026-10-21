#include "pch.h"
#include "WebCamCapture.h"
// pch.h/WIL defines GUID_NULL as a __uuidof expression. WRL includes
// cguid.h, which declares a GUID_NULL variable. Hide only this macro while
// parsing those SDK declarations, then restore the project's definition.
#pragma push_macro("GUID_NULL")
#undef GUID_NULL
#include <cguid.h>
#include <wrl/implements.h>
#include <wrl/ftm.h>
#pragma pop_macro("GUID_NULL")
#include <d3d10.h>
#include <d3d11.h>
#include <algorithm>
#include <chrono>
#include <condition_variable>
#include <cwctype>
#include <mutex>
#include <new>
#include <string>

using Microsoft::WRL::ComPtr;

// Owned independently of WebcamCapture, so late reader callbacks cannot touch a
// destroyed capture object. At most one asynchronous read is outstanding.
struct WebcamReadState
{
    std::mutex mutex;
    std::condition_variable ready;
    bool stopped = false;
    bool complete = false;
    HRESULT status = S_OK;
    DWORD flags = 0;
    LONGLONG timestamp = 0;
    ComPtr<IMFSample> sample;
};

namespace
{
    class ReaderCallback final : public Microsoft::WRL::RuntimeClass<
        Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>, IMFSourceReaderCallback, Microsoft::WRL::FtmBase>
    {
    public:
        explicit ReaderCallback(std::shared_ptr<WebcamReadState> state) : _state(std::move(state)) {}

        STDMETHODIMP OnReadSample(HRESULT status, DWORD, DWORD flags,
            LONGLONG timestamp, IMFSample* sample) override
        {
            std::lock_guard<std::mutex> lock(_state->mutex);
            if (!_state->stopped)
            {
                _state->status = status;
                _state->flags = flags;
                _state->timestamp = timestamp;
                _state->sample = sample;
                _state->complete = true;
                _state->ready.notify_all();
            }
            return S_OK;
        }
        STDMETHODIMP OnFlush(DWORD) override { return S_OK; }
        STDMETHODIMP OnEvent(DWORD, IMFMediaEvent*) override { return S_OK; }
    private:
        std::shared_ptr<WebcamReadState> _state;
    };

    bool IsVirtualCamera(IMFActivate* device, const std::wstring& displayName)
    {
        // Do not let Ascii Cam capture itself. Names alone are insufficient:
        // only accept devices which Windows explicitly marks as hardware.
        UINT32 hardware = FALSE;
        if (FAILED(device->GetUINT32(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_HW_SOURCE, &hardware)) || !hardware)
            return true;
        std::wstring name = displayName;
        std::transform(name.begin(), name.end(), name.begin(), [](wchar_t c) { return std::towlower(c); });
        for (const auto* marker : { L"virtual", L"ascii cam", L"ascii-cam", L"obs camera",
            L"manycam", L"snap camera", L"xsplit", L"splitcam", L"mmhmm" })
            if (name.find(marker) != std::wstring::npos) return true;
        return false;
    }
}

WebcamCapture::WebcamCapture() = default;
WebcamCapture::~WebcamCapture() { Shutdown(); }

HRESULT WebcamCapture::Initialize(UINT width, UINT height, UINT fps)
{
    Shutdown();
    RETURN_HR_IF(E_INVALIDARG, !width || !height || (width % 2) || (height % 2) || !fps);
    _width = width;
    _height = height;
    wil::com_ptr_nothrow<IMFAttributes> attributes;
    RETURN_IF_FAILED(MFCreateAttributes(&attributes, 1));
    RETURN_IF_FAILED(attributes->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
        MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID));

    IMFActivate** devices = nullptr;
    UINT32 count = 0;
    RETURN_IF_FAILED(MFEnumDeviceSources(attributes.get(), &devices, &count));
    HRESULT activationResult = MF_E_NOT_FOUND;
    for (UINT32 i = 0; i < count; ++i)
    {
        WCHAR* name = nullptr;
        UINT32 length = 0;
        if (SUCCEEDED(devices[i]->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &name, &length)))
        {
            WinTraceFormat(4, 0, L"[Wcam] Found: %s", name);
            if (!_source && !IsVirtualCamera(devices[i], name))
            {
                activationResult = devices[i]->ActivateObject(IID_PPV_ARGS(&_source));
                if (SUCCEEDED(activationResult)) WinTraceFormat(4, 0, L"[Wcam] Selected physical camera: %s", name);
            }
            CoTaskMemFree(name);
        }
        devices[i]->Release();
    }
    CoTaskMemFree(devices);
    RETURN_HR_IF(activationResult, !_source);

    // Require a real D3D11 device; no WARP or CPU retry for video processing.
    ComPtr<ID3D11Device> device;
    ComPtr<ID3D11DeviceContext> context;
    D3D_FEATURE_LEVEL level;
    RETURN_IF_FAILED(D3D11CreateDevice(nullptr, D3D_DRIVER_TYPE_HARDWARE, nullptr,
        D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
        nullptr, 0, D3D11_SDK_VERSION, &device, &level, &context));
    ComPtr<ID3D10Multithread> multithread;
    RETURN_IF_FAILED(device.As(&multithread));
    multithread->SetMultithreadProtected(TRUE);
    UINT token = 0;
    ComPtr<IMFDXGIDeviceManager> manager;
    RETURN_IF_FAILED(MFCreateDXGIDeviceManager(&token, &manager));
    RETURN_IF_FAILED(manager->ResetDevice(device.Get(), token));

    try { _readState = std::make_shared<WebcamReadState>(); }
    catch (const std::bad_alloc&) { return E_OUTOFMEMORY; }
    auto callback = Microsoft::WRL::Make<ReaderCallback>(_readState);
    RETURN_HR_IF(E_OUTOFMEMORY, !callback);
    RETURN_IF_FAILED(callback.As(&_callback));
    wil::com_ptr_nothrow<IMFAttributes> readerAttributes;
    RETURN_IF_FAILED(MFCreateAttributes(&readerAttributes, 4));
    RETURN_IF_FAILED(readerAttributes->SetUINT32(MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, TRUE));
    RETURN_IF_FAILED(readerAttributes->SetUINT32(MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, TRUE));
    RETURN_IF_FAILED(readerAttributes->SetUnknown(MF_SOURCE_READER_D3D_MANAGER, manager.Get()));
    RETURN_IF_FAILED(readerAttributes->SetUnknown(MF_SOURCE_READER_ASYNC_CALLBACK, _callback.Get()));
    RETURN_IF_FAILED(MFCreateSourceReaderFromMediaSource(_source.Get(), readerAttributes.get(), &_reader));
    RETURN_IF_FAILED(_reader->SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS, FALSE));
    RETURN_IF_FAILED(_reader->SetStreamSelection(MF_SOURCE_READER_FIRST_VIDEO_STREAM, TRUE));

    // Choose an exact-size mode. Prefer native NV12, then uncompressed YUY2;
    // never silently feed a different resolution or packed YUY2 into the filter.
    DWORD bestIndex = MAXDWORD;
    UINT32 bestScore = 0;
    for (DWORD index = 0; ; ++index)
    {
        ComPtr<IMFMediaType> native;
        const HRESULT hr = _reader->GetNativeMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM, index, &native);
        if (hr == MF_E_NO_MORE_TYPES) break;
        RETURN_IF_FAILED(hr);
        UINT32 w = 0, h = 0, num = 0, den = 0;
        GUID format = GUID_NULL;
        if (FAILED(MFGetAttributeSize(native.Get(), MF_MT_FRAME_SIZE, &w, &h)) ||
            FAILED(MFGetAttributeRatio(native.Get(), MF_MT_FRAME_RATE, &num, &den)) ||
            FAILED(native->GetGUID(MF_MT_SUBTYPE, &format)) || !den || !num || w != width || h != height ||
            static_cast<UINT64>(num) != static_cast<UINT64>(fps) * den)
            continue;
        const UINT32 rate = num / den;
        // Favor the requested rate, then closest rate, within the format tier.
        const UINT32 difference = rate > fps ? rate - fps : fps - rate;
        const UINT32 rateScore = 1000 - (std::min)(difference, 999u);
        const UINT32 formatScore = format == MFVideoFormat_NV12 ? 3000 :
            format == MFVideoFormat_YUY2 ? 2000 : 1000;
        const UINT32 score = formatScore + rateScore;
        if (bestIndex == MAXDWORD || score > bestScore) { bestIndex = index; bestScore = score; }
    }
    if (bestIndex == MAXDWORD)
    {
        WinTraceFormat(4, 0, L"[Wcam] No native %ux%u @ %u fps mode. Refusing a mismatched fallback.", width, height, fps);
        return MF_E_INVALIDMEDIATYPE;
    }
    ComPtr<IMFMediaType> native;
    RETURN_IF_FAILED(_reader->GetNativeMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM, bestIndex, &native));
    RETURN_IF_FAILED(_reader->SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM, nullptr, native.Get()));

    ComPtr<IMFMediaType> output;
    RETURN_IF_FAILED(MFCreateMediaType(&output));
    RETURN_IF_FAILED(output->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video));
    RETURN_IF_FAILED(output->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_NV12));
    RETURN_IF_FAILED(MFSetAttributeSize(output.Get(), MF_MT_FRAME_SIZE, width, height));
    RETURN_IF_FAILED(output->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive));
    // Leave frame rate to the native mode; its timestamps/durations are retained.
    RETURN_IF_FAILED(_reader->SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM, nullptr, output.Get()));
    ComPtr<IMFMediaType> actual;
    RETURN_IF_FAILED(GetOutputType(&actual));
    GUID format = GUID_NULL;
    UINT32 w = 0, h = 0;
    RETURN_IF_FAILED(actual->GetGUID(MF_MT_SUBTYPE, &format));
    RETURN_IF_FAILED(MFGetAttributeSize(actual.Get(), MF_MT_FRAME_SIZE, &w, &h));
    RETURN_HR_IF(MF_E_INVALIDMEDIATYPE, format != MFVideoFormat_NV12 || w != width || h != height);
    WinTraceFormat(4, 0, L"[Wcam] NV12 output verified: %ux%u; D3D11 enabled.", w, h);
    return S_OK;
}

HRESULT WebcamCapture::GetOutputType(IMFMediaType** type)
{
    RETURN_HR_IF_NULL(E_POINTER, type);
    *type = nullptr;
    RETURN_HR_IF(E_UNEXPECTED, !_reader);
    return _reader->GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM, type);
}

HRESULT WebcamCapture::GetFrame(IMFSample** sample)
{
    RETURN_HR_IF_NULL(E_POINTER, sample);
    *sample = nullptr;
    RETURN_HR_IF(E_UNEXPECTED, !_reader || !_readState);
    auto state = _readState;
    {
        std::lock_guard<std::mutex> lock(state->mutex);
        if (state->stopped) return MF_E_SHUTDOWN;
        state->complete = false;
        state->sample.Reset();
    }
    RETURN_IF_FAILED(_reader->ReadSample(MF_SOURCE_READER_FIRST_VIDEO_STREAM, 0,
        nullptr, nullptr, nullptr, nullptr));
    std::unique_lock<std::mutex> lock(state->mutex);
    if (!state->ready.wait_for(lock, std::chrono::seconds(5), [&] { return state->complete || state->stopped; }))
        return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
    if (state->stopped) return MF_E_SHUTDOWN;
    RETURN_IF_FAILED(state->status);
    if (state->flags & MF_SOURCE_READERF_ERROR) return E_FAIL;
    if (state->flags & MF_SOURCE_READERF_ENDOFSTREAM) return MF_E_END_OF_STREAM;
    const DWORD flags = state->flags;
    const LONGLONG timestamp = state->timestamp;
    auto frame = state->sample;
    lock.unlock();
    if (flags & (MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED | MF_SOURCE_READERF_NATIVEMEDIATYPECHANGED))
    {
        ComPtr<IMFMediaType> type;
        RETURN_IF_FAILED(GetOutputType(&type));
        GUID format = GUID_NULL;
        UINT32 width = 0, height = 0, num = 0, den = 0;
        RETURN_IF_FAILED(type->GetGUID(MF_MT_SUBTYPE, &format));
        RETURN_IF_FAILED(MFGetAttributeSize(type.Get(), MF_MT_FRAME_SIZE, &width, &height));
        RETURN_IF_FAILED(MFGetAttributeRatio(type.Get(), MF_MT_FRAME_RATE, &num, &den));
        RETURN_HR_IF(MF_E_INVALIDMEDIATYPE, format != MFVideoFormat_NV12 ||
            width != _width || height != _height || !den || static_cast<UINT64>(num) != 30ull * den);
    }
    // Stream ticks are valid and carry no sample. The next read may have one.
    if (!frame) return S_FALSE;
    RETURN_IF_FAILED(frame->SetSampleTime(timestamp));
    return frame.CopyTo(sample);
}

void WebcamCapture::RequestStop()
{
    if (_readState)
    {
        std::lock_guard<std::mutex> lock(_readState->mutex);
        _readState->stopped = true;
        _readState->ready.notify_all();
    }
    // Do not wait for a driver read to finish just to join the capture thread.
    // Flush is asynchronous when the reader has a callback.
    if (_reader) (void)_reader->Flush(MF_SOURCE_READER_ALL_STREAMS);
}

void WebcamCapture::Shutdown()
{
    RequestStop();
    _reader.Reset();
    if (_source) { (void)_source->Shutdown(); _source.Reset(); }
    _callback.Reset();
    _readState.reset();
}
