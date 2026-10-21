#pragma once

#include "WebCamCapture.h"
#include "ColorConverter.h"
#include "FrameGenerator.h"
#include "Filter.h"
#include <winerror.h>
#include <mferror.h>
#include <thread>
#include <deque>
#include <mutex>
#include <optional>

struct MediaStream :
    winrt::implements<
    MediaStream,
    CBaseAttributes<IMFAttributes>,
    IMFMediaStream2,
    IKsControl>
{
public:
    // IMFMediaEventGenerator
    STDMETHOD(BeginGetEvent)(
        IMFAsyncCallback* pCallback,
        IUnknown* punkState
        );

    STDMETHOD(EndGetEvent)(
        IMFAsyncResult* pResult,
        IMFMediaEvent** ppEvent
        );

    STDMETHOD(GetEvent)(
        DWORD dwFlags,
        IMFMediaEvent** ppEvent
        );

    STDMETHOD(QueueEvent)(
        MediaEventType met,
        REFGUID guidExtendedType,
        HRESULT hrStatus,
        const PROPVARIANT* pvValue
        );

    // IMFMediaStream
    STDMETHOD(GetMediaSource)(
        IMFMediaSource** ppMediaSource
        );

    STDMETHOD(GetStreamDescriptor)(
        IMFStreamDescriptor** ppStreamDescriptor
        );

    STDMETHOD(RequestSample)(
        IUnknown* pToken
        );

    // IMFMediaStream2
    STDMETHOD(SetStreamState)(
        MF_STREAM_STATE value
        );

    STDMETHOD(GetStreamState)(
        MF_STREAM_STATE* value
        );

    // IKsControl
    STDMETHOD_(NTSTATUS, KsProperty)(
        PKSPROPERTY Property,
        ULONG PropertyLength,
        LPVOID PropertyData,
        ULONG DataLength,
        ULONG* BytesReturned
        );

    STDMETHOD_(NTSTATUS, KsMethod)(
        PKSMETHOD Method,
        ULONG MethodLength,
        LPVOID MethodData,
        ULONG DataLength,
        ULONG* BytesReturned
        );

    STDMETHOD_(NTSTATUS, KsEvent)(
        PKSEVENT Event,
        ULONG EventLength,
        LPVOID EventData,
        ULONG DataLength,
        ULONG* BytesReturned
        );

public:

    MediaStream() :
        _index(0),
        _state(MF_STREAM_STATE_STOPPED),
        _format(GUID_NULL)
    {
        SetBaseAttributesTraceName(L"MediaStreamAtts");
    }

    ~MediaStream() { Shutdown(); }

    HRESULT Initialize(
        IMFMediaSource* source,
        int index
    );

    HRESULT SetAllocator(
        IUnknown* allocator
    );

    MFSampleAllocatorUsage GetAllocatorUsage();

    HRESULT SetD3DManager(
        IUnknown* manager
    );

    HRESULT Start(
        IMFMediaType* type
    );

    HRESULT Stop();

    void Shutdown();

private:

    void CaptureLoop(std::stop_token stopToken);
    // Called only with _lock held. Requests are FIFO, including null tokens.
    HRESULT DeliverSample(IMFSample* frame, IUnknown* token);
    void ReportStreamError(HRESULT error);
    void StopCapture();

#if _DEBUG

    int32_t query_interface_tearoff(
        winrt::guid const& id,
        void** object
    ) const noexcept override
    {
        RETURN_HR_MSG(
            E_NOINTERFACE,
            "MediaStream QueryInterface failed on IID %s",
            GUID_ToStringW(id).c_str()
        );
    }

#endif

private:

    // webcam capture
    WebcamCapture _capture;

    // color converter
    ColorConverter _converter;
    // capture thread
    std::jthread _captureThread;

    Filter _postProcessor;

    // Serializes lifecycle calls; never held by the capture worker.
    std::mutex _lifecycleMutex;
    bool _allocatorInitialized = false;
    bool _shutdown = false;
    HRESULT _streamError = S_OK;
    UINT32 _width = 1280;
    UINT32 _height = 720;
    LONGLONG _duration = 333333;
    LONGLONG _baseTime = 0;
    std::optional<LONGLONG> _firstCaptureTime;
    std::optional<LONGLONG> _lastDeliveredTime;
    std::deque<Microsoft::WRL::ComPtr<IUnknown>> _requests;

    // latest webcam frame
    wil::com_ptr_nothrow<IMFSample> _latestFrame;

    // existing MF lock
    winrt::slim_mutex _lock;

    MF_STREAM_STATE _state;

    FrameGenerator _generator;

    GUID _format;

    wil::com_ptr_nothrow<IMFStreamDescriptor> _descriptor;

    wil::com_ptr_nothrow<IMFMediaEventQueue> _queue;

    wil::com_ptr_nothrow<IMFMediaSource> _source;

    wil::com_ptr_nothrow<IMFVideoSampleAllocatorEx> _allocator;

    int _index;
};