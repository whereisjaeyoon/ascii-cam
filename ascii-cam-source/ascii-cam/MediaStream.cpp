#include "pch.h"
#include "Undocumented.h"
#include "Tools.h"
#include "EnumNames.h"
#include "MFTools.h"
#include "MediaStream.h"
#include "MediaSource.h"
#include "Nv12Layout.h"
#include <vector>
#include <new>
#include <system_error>

#define NUM_IMAGE_COLS 1280
#define NUM_IMAGE_ROWS 720

HRESULT MediaStream::Initialize(IMFMediaSource* source, int index)
{
    RETURN_HR_IF_NULL(E_POINTER, source);
    _source = source;
    _index = index;
    RETURN_IF_FAILED(SetGUID(MF_DEVICESTREAM_STREAM_CATEGORY, PINNAME_VIDEO_CAPTURE));
    RETURN_IF_FAILED(SetUINT32(MF_DEVICESTREAM_STREAM_ID, index));
    RETURN_IF_FAILED(SetUINT32(MF_DEVICESTREAM_FRAMESERVER_SHARED, 1));
    RETURN_IF_FAILED(SetUINT32(MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES, MFFrameSourceTypes_Color));
    RETURN_IF_FAILED(MFCreateEventQueue(&_queue));

    // Until an RGB32 conversion is implemented, advertise ONLY the format that
    // is actually delivered. NV12 stride is width, not width * 1.5.
    wil::com_ptr_nothrow<IMFMediaType> type;
    RETURN_IF_FAILED(MFCreateMediaType(&type));
    RETURN_IF_FAILED(type->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video));
    RETURN_IF_FAILED(type->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_NV12));
    RETURN_IF_FAILED(MFSetAttributeSize(type.get(), MF_MT_FRAME_SIZE, NUM_IMAGE_COLS, NUM_IMAGE_ROWS));
    RETURN_IF_FAILED(type->SetUINT32(MF_MT_DEFAULT_STRIDE, NUM_IMAGE_COLS));
    RETURN_IF_FAILED(type->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive));
    RETURN_IF_FAILED(type->SetUINT32(MF_MT_ALL_SAMPLES_INDEPENDENT, TRUE));
    RETURN_IF_FAILED(type->SetUINT32(MF_MT_FIXED_SIZE_SAMPLES, TRUE));
    RETURN_IF_FAILED(type->SetUINT32(MF_MT_SAMPLE_SIZE, NUM_IMAGE_COLS * NUM_IMAGE_ROWS * 3 / 2));
    RETURN_IF_FAILED(type->SetUINT32(MF_MT_AVG_BITRATE, NUM_IMAGE_COLS * NUM_IMAGE_ROWS * 12 * 30));
    RETURN_IF_FAILED(MFSetAttributeRatio(type.get(), MF_MT_FRAME_RATE, 30, 1));
    RETURN_IF_FAILED(MFSetAttributeRatio(type.get(), MF_MT_PIXEL_ASPECT_RATIO, 1, 1));
    IMFMediaType* types[] = { type.get() };
    RETURN_IF_FAILED(MFCreateStreamDescriptor(index, 1, types, &_descriptor));
    wil::com_ptr_nothrow<IMFMediaTypeHandler> handler;
    RETURN_IF_FAILED(_descriptor->GetMediaTypeHandler(&handler));
    RETURN_IF_FAILED(handler->SetCurrentMediaType(type.get()));
    _format = MFVideoFormat_NV12;
    return S_OK;
}

HRESULT MediaStream::Start(IMFMediaType* requestedType)
{
	RETURN_HR_IF(MF_E_SHUTDOWN, !_queue || !_allocator);
	UINT32 width = 1280, height = 720;
	UINT32 fpsNumerator = 30, fpsDenominator = 1;
	if (type)
	{
		RETURN_IF_FAILED(type->GetGUID(MF_MT_SUBTYPE, &_format));
		MFGetAttributeSize(type, MF_MT_FRAME_SIZE, &width, &height);
		MFGetAttributeRatio(type, MF_MT_FRAME_RATE, &fpsNumerator, &fpsDenominator);
		WINTRACE(L"MediaStream::Start - Format: %s, Resolution: %dx%d, FPS: %d",
			GUID_ToStringW(_format).c_str(), width, height,
			(fpsDenominator > 0 ? fpsNumerator / fpsDenominator : 30));
	}
	UINT32 framerate = (fpsDenominator > 0) ? (fpsNumerator / fpsDenominator) : 30;
	RETURN_IF_FAILED(_capture.Initialize(width, height, framerate));

	RETURN_IF_FAILED(_postProcessor.Initialize(width, height, FilterOption::Grayscale));

	RETURN_IF_FAILED(
		_allocator->InitializeSampleAllocator(
			10,
			type
		)
	);

	_state = MF_STREAM_STATE_RUNNING;
	HRESULT hr = _queue->QueueEventParamVar(
		MEStreamStarted,
		GUID_NULL,
		S_OK,
		nullptr
	);
	if (FAILED(hr))
	{
		// 이벤트 발송 실패 시 롤백 처리
		_state = MF_STREAM_STATE_STOPPED;
		_postProcessor.Shutdown();
		_capture.Shutdown();
		return hr;
	}
	_captureThread = std::jthread([this](std::stop_token stopToken) {
		CaptureLoop(stopToken);
		});

	return S_OK;

// Caller holds the lifecycle mutex, never the event/sample mutex during join.
void MediaStream::StopCapture()
{
    if (_captureThread.joinable())
    {
        _captureThread.request_stop();
        _capture.RequestStop();
        _captureThread.join();
    }
    _capture.Shutdown();
    _postProcessor.Shutdown();
    if (_allocatorInitialized && _allocator)
    {
        LOG_IF_FAILED(_allocator->UninitializeSampleAllocator());
        _allocatorInitialized = false;
    }
}

HRESULT MediaStream::Stop()
{
    std::lock_guard<std::mutex> lifecycle(_lifecycleMutex);
    {
        winrt::slim_lock_guard lock(_lock);
        RETURN_HR_IF(MF_E_SHUTDOWN, _shutdown || !_queue);
        _state = MF_STREAM_STATE_STOPPED;
        _requests.clear();
        _latestFrame.reset();
    }
    StopCapture();
    winrt::slim_lock_guard lock(_lock);
    return _queue->QueueEventParamVar(MEStreamStopped, GUID_NULL, S_OK, nullptr);
}

void MediaStream::ReportStreamError(HRESULT error)
{
	WINTRACE(L"CaptureLoop Started");
	while (!stopToken.stop_requested() && _state == MF_STREAM_STATE_RUNNING)
	{
		wil::com_ptr_nothrow<IMFSample> webcamSample;

		// 프레임 캡처
		HRESULT hr = _capture.GetFrame(&webcamSample);

		if (FAILED(hr) || !webcamSample)
		{
			WINTRACE(L"GetFrame Failed: 0x%08X", hr);

			// 장치가 끊겼거나 심각한 실패일 경우 루프 탈출 고려
			if (hr == MF_E_VIDEO_RECORDING_DEVICE_INVALIDATED ||
				hr == HRESULT_FROM_WIN32(ERROR_DEVICE_NOT_CONNECTED) ||
				hr == E_HANDLE)
			{
				WINTRACE(L"Webcam device disconnected. (0x%08X)", hr);
				break;
			}

			continue;
		}

		FilterOption requested;
		if (_filterSubscriber.TryGetUpdate(requested))
		{
			HRESULT setHr = _postProcessor.SetOption(requested);
			WINTRACE(L"Filter switched to %d: 0x%08X", (int)requested, setHr);
			// Acknowledge only after the switch finished (even on failure, so the producer unblocks).
			_filterSubscriber.NotifyConsumed();
		}

		hr = _postProcessor.Process(webcamSample.get());
		if (FAILED(hr))
		{
			WINTRACE(L"Filter Process Failed: 0x%08X", hr);
			continue;
		}

		// 최신 프레임 스레드 안전 업데이트
		{
			winrt::slim_lock_guard guard(_frameLock);
			_latestFrame = std::move(webcamSample); // std::move로 Ref-Count 증감 오버헤드 방지
		}
	}

	// 스레드 종료 시 캡처 장치 자원 해제
	_capture.Shutdown();

void MediaStream::CaptureLoop(std::stop_token token)
{
    const HRESULT comResult = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    if (FAILED(comResult))
    {
        winrt::slim_lock_guard lock(_lock);
        ReportStreamError(comResult);
        return;
    }
    try
    {
        while (!token.stop_requested())
        {
            wil::com_ptr_nothrow<IMFSample> frame;
            HRESULT hr = _capture.GetFrame(&frame);
            if (token.stop_requested()) break;
            if (hr == S_FALSE) continue;
            if (SUCCEEDED(hr)) hr = _postProcessor.Process(frame.get());
            winrt::slim_lock_guard lock(_lock);
            if (token.stop_requested() || _shutdown || _state == MF_STREAM_STATE_STOPPED) break;
            if (FAILED(hr)) { ReportStreamError(hr); break; }
            _latestFrame = std::move(frame);
            if (_state == MF_STREAM_STATE_RUNNING && !_requests.empty())
            {
                hr = DeliverSample(_latestFrame.get(), _requests.front().Get());
                if (FAILED(hr)) { ReportStreamError(hr); break; }
                _requests.pop_front();
                _latestFrame.reset();
            }
        }
    }
    catch (const std::bad_alloc&)
    {
        winrt::slim_lock_guard lock(_lock);
        ReportStreamError(E_OUTOFMEMORY);
    }
    CoUninitialize();
}

MFSampleAllocatorUsage MediaStream::GetAllocatorUsage()
{
    return MFSampleAllocatorUsage_UsesProvidedAllocator;
}

HRESULT MediaStream::SetAllocator(IUnknown* allocator)
{
    RETURN_HR_IF_NULL(E_POINTER, allocator);
    std::lock_guard<std::mutex> lifecycle(_lifecycleMutex);
    winrt::slim_lock_guard lock(_lock);
    RETURN_HR_IF(MF_E_SHUTDOWN, _shutdown);
    RETURN_HR_IF(MF_E_INVALIDREQUEST, _allocatorInitialized);
    return allocator->QueryInterface(IID_PPV_ARGS(&_allocator));
}

HRESULT MediaStream::SetD3DManager(IUnknown* manager)
{
    RETURN_HR_IF_NULL(E_POINTER, manager);
    std::lock_guard<std::mutex> lifecycle(_lifecycleMutex);
    winrt::slim_lock_guard lock(_lock);
    RETURN_HR_IF(MF_E_SHUTDOWN, _shutdown || !_allocator);
    // FrameGenerator is unused in the live path. Do not initialize its unrelated
    // D2D resources while configuring the sample allocator.
    return _allocator->SetDirectXManager(manager);
}

void MediaStream::Shutdown()
{
    std::lock_guard<std::mutex> lifecycle(_lifecycleMutex);
    {
        winrt::slim_lock_guard lock(_lock);
        if (_shutdown) return;
        _shutdown = true;
        _state = MF_STREAM_STATE_STOPPED;
        _requests.clear();
        _latestFrame.reset();
    }
    StopCapture();
    winrt::slim_lock_guard lock(_lock);
    if (_queue) { (void)_queue->Shutdown(); _queue.reset(); }
    _descriptor.reset();
    _source.reset();
    _allocator.reset();
    _attributes.reset();
}

// Snapshot the queue before calling potentially blocking event operations.
// Holding _lock inside GetEvent would prevent the capture worker from delivering.
STDMETHODIMP MediaStream::BeginGetEvent(IMFAsyncCallback* callback, IUnknown* state)
{
    wil::com_ptr_nothrow<IMFMediaEventQueue> queue;
    { winrt::slim_lock_guard lock(_lock); queue = _queue; }
    RETURN_HR_IF(MF_E_SHUTDOWN, !queue);
    return queue->BeginGetEvent(callback, state);
}
STDMETHODIMP MediaStream::EndGetEvent(IMFAsyncResult* result, IMFMediaEvent** event)
{
    RETURN_HR_IF_NULL(E_POINTER, event);
    *event = nullptr;
    wil::com_ptr_nothrow<IMFMediaEventQueue> queue;
    { winrt::slim_lock_guard lock(_lock); queue = _queue; }
    RETURN_HR_IF(MF_E_SHUTDOWN, !queue);
    return queue->EndGetEvent(result, event);
}
STDMETHODIMP MediaStream::GetEvent(DWORD flags, IMFMediaEvent** event)
{
    RETURN_HR_IF_NULL(E_POINTER, event);
    *event = nullptr;
    wil::com_ptr_nothrow<IMFMediaEventQueue> queue;
    { winrt::slim_lock_guard lock(_lock); queue = _queue; }
    RETURN_HR_IF(MF_E_SHUTDOWN, !queue);
    return queue->GetEvent(flags, event);
}
STDMETHODIMP MediaStream::QueueEvent(MediaEventType type, REFGUID extended, HRESULT status, const PROPVARIANT* value)
{
    wil::com_ptr_nothrow<IMFMediaEventQueue> queue;
    { winrt::slim_lock_guard lock(_lock); queue = _queue; }
    RETURN_HR_IF(MF_E_SHUTDOWN, !queue);
    return queue->QueueEventParamVar(type, extended, status, value);
}
STDMETHODIMP MediaStream::GetMediaSource(IMFMediaSource** source)
{
    RETURN_HR_IF_NULL(E_POINTER, source);
    *source = nullptr;
    winrt::slim_lock_guard lock(_lock);
    RETURN_HR_IF(MF_E_SHUTDOWN, _shutdown || !_source);
    return _source.copy_to(source);
}
STDMETHODIMP MediaStream::GetStreamDescriptor(IMFStreamDescriptor** descriptor)
{
    RETURN_HR_IF_NULL(E_POINTER, descriptor);
    *descriptor = nullptr;
    winrt::slim_lock_guard lock(_lock);
    RETURN_HR_IF(MF_E_SHUTDOWN, _shutdown || !_descriptor);
    return _descriptor.copy_to(descriptor);
}

HRESULT MediaStream::DeliverSample(IMFSample* frame, IUnknown* token)
{
    RETURN_HR_IF_NULL(E_POINTER, frame);
    wil::com_ptr_nothrow<IMFMediaBuffer> input;
    RETURN_IF_FAILED(frame->ConvertToContiguousBuffer(&input));
    Nv12Layout packedLayout;
    RETURN_HR_IF(MF_E_INVALIDMEDIATYPE, !Nv12Layout::TryCreate(_width, _height, _width, packedLayout));
    const DWORD packedLength = packedLayout.packedBytes;
    std::vector<BYTE> packed(packedLength);
    wil::com_ptr_nothrow<IMF2DBuffer> input2D;
    if (SUCCEEDED(input->QueryInterface(IID_PPV_ARGS(&input2D))))
    {
        RETURN_IF_FAILED(input2D->ContiguousCopyTo(packed.data(), packedLength));
    }
    else
    {
        BYTE* bytes = nullptr;
        DWORD length = 0;
        RETURN_IF_FAILED(input->Lock(&bytes, nullptr, &length));
        // For linear buffers, use the actual capture stride, not output stride.
        wil::com_ptr_nothrow<IMFMediaType> captureType;
        HRESULT hr = _capture.GetOutputType(&captureType);
        const UINT32 stride = SUCCEEDED(hr) ? MFGetAttributeUINT32(captureType.get(), MF_MT_DEFAULT_STRIDE, _width) : 0;
        if (SUCCEEDED(hr))
        {
            Nv12Layout inputLayout;
            if (!Nv12Layout::TryCreate(_width, _height, stride, inputLayout) || inputLayout.footprint > length)
                hr = MF_E_BUFFERTOOSMALL;
            else hr = MFCopyImage(packed.data(), _width, bytes, static_cast<LONG>(stride), _width, _height * 3 / 2);
        }
        const HRESULT unlock = input->Unlock();
        RETURN_IF_FAILED(hr);
        RETURN_IF_FAILED(unlock);
    }

    // Honor UsesProvidedAllocator, including DXGI surfaces supplied by FrameServer.
    wil::com_ptr_nothrow<IMFSample> output;
    RETURN_IF_FAILED(_allocator->AllocateSample(&output));
    wil::com_ptr_nothrow<IMFMediaBuffer> destination;
    RETURN_IF_FAILED(output->GetBufferByIndex(0, &destination));
    wil::com_ptr_nothrow<IMF2DBuffer> destination2D;
    if (SUCCEEDED(destination->QueryInterface(IID_PPV_ARGS(&destination2D))))
    {
        RETURN_IF_FAILED(destination2D->ContiguousCopyFrom(packed.data(), packedLength));
    }
    else
    {
        BYTE* bytes = nullptr;
        DWORD capacity = 0;
        RETURN_IF_FAILED(destination->Lock(&bytes, &capacity, nullptr));
        const HRESULT hr = capacity < packedLength ? MF_E_BUFFERTOOSMALL :
            MFCopyImage(bytes, _width, packed.data(), _width, _width, _height * 3 / 2);
        const HRESULT unlock = destination->Unlock();
        RETURN_IF_FAILED(hr);
        RETURN_IF_FAILED(unlock);
    }
    RETURN_IF_FAILED(destination->SetCurrentLength(packedLength));
    LONGLONG duration = _duration, captureTime = 0;
    (void)frame->GetSampleDuration(&duration);
    if (duration <= 0) duration = _duration;
    RETURN_IF_FAILED(frame->GetSampleTime(&captureTime));
    if (!_firstCaptureTime) _firstCaptureTime = captureTime;
    LONGLONG timestamp = _baseTime + (captureTime - *_firstCaptureTime);
    if (_lastDeliveredTime && timestamp <= *_lastDeliveredTime)
        timestamp = *_lastDeliveredTime + duration;
    RETURN_IF_FAILED(output->SetSampleTime(timestamp));
    RETURN_IF_FAILED(output->SetSampleDuration(duration));
    // Allocator samples can be recycled; always remove any old request token.
    (void)output->DeleteItem(MFSampleExtension_Token);
    RETURN_IF_FAILED(output->SetUINT32(MFSampleExtension_Discontinuity, !_lastDeliveredTime.has_value()));
    if (token) RETURN_IF_FAILED(output->SetUnknown(MFSampleExtension_Token, token));
    RETURN_IF_FAILED(_queue->QueueEventParamUnk(MEMediaSample, GUID_NULL, S_OK, output.get()));
    if (!_lastDeliveredTime) WinTraceFormat(4, 0, L"[AsciiCam] First NV12 sample delivered (%u bytes).", packedLength);
    _lastDeliveredTime = timestamp;
    return S_OK;
}

STDMETHODIMP MediaStream::RequestSample(IUnknown* token)
{
    winrt::slim_lock_guard lock(_lock);
    RETURN_HR_IF(MF_E_SHUTDOWN, _shutdown || !_queue || !_allocator);
    RETURN_HR_IF(MF_E_INVALIDREQUEST, _state == MF_STREAM_STATE_STOPPED);
    RETURN_IF_FAILED(_streamError);
    // Never return S_OK without remembering the request. A first-frame race
    // previously discarded this request and left the client waiting forever.
    RETURN_HR_IF(MF_E_NOTACCEPTING, _requests.size() >= 32);
    try
    {
        _requests.emplace_back(token);
        if (_state == MF_STREAM_STATE_RUNNING && _latestFrame)
        {
            const HRESULT hr = DeliverSample(_latestFrame.get(), _requests.front().Get());
            if (FAILED(hr)) { ReportStreamError(hr); return hr; }
            _requests.pop_front();
            _latestFrame.reset();
        }
    }
    catch (const std::bad_alloc&)
    {
        ReportStreamError(E_OUTOFMEMORY);
        return E_OUTOFMEMORY;
    }
    return S_OK;
}

STDMETHODIMP MediaStream::SetStreamState(MF_STREAM_STATE value)
{
    if (value == MF_STREAM_STATE_RUNNING) return Start(nullptr);
    if (value == MF_STREAM_STATE_STOPPED) return Stop();
    std::lock_guard<std::mutex> lifecycle(_lifecycleMutex);
    winrt::slim_lock_guard lock(_lock);
    RETURN_HR_IF(MF_E_SHUTDOWN, _shutdown || !_queue);
    if (_state == value) return S_OK; // Comparison, not assignment.
    RETURN_HR_IF(MF_E_INVALID_STATE_TRANSITION,
        value != MF_STREAM_STATE_PAUSED || _state != MF_STREAM_STATE_RUNNING);
    _state = value;
    return _queue->QueueEventParamVar(MEStreamPaused, GUID_NULL, S_OK, nullptr);
}
STDMETHODIMP MediaStream::GetStreamState(MF_STREAM_STATE* value)
{
    RETURN_HR_IF_NULL(E_POINTER, value);
    winrt::slim_lock_guard lock(_lock);
    RETURN_HR_IF(MF_E_SHUTDOWN, _shutdown);
    *value = _state;
    return S_OK;
}

// IKsControl
STDMETHODIMP_(NTSTATUS) MediaStream::KsProperty(PKSPROPERTY property, ULONG length, LPVOID data, ULONG dataLength, ULONG* bytesReturned)
{
	WINTRACE(L"MediaStream::KsProperty len:%u data:%p dataLength:%u", length, data, dataLength);
	RETURN_HR_IF_NULL(E_POINTER, property);
	RETURN_HR_IF_NULL(E_POINTER, bytesReturned);
	winrt::slim_lock_guard lock(_lock);

	WINTRACE(L"MediaStream::KsProperty prop:%s", PKSIDENTIFIER_ToString(property, length).c_str());

	return HRESULT_FROM_WIN32(ERROR_SET_NOT_FOUND);
}

STDMETHODIMP_(NTSTATUS) MediaStream::KsMethod(PKSMETHOD method, ULONG length, LPVOID data, ULONG dataLength, ULONG* bytesReturned)
{
	WINTRACE(L"MediaStream::KsMethod len:%u data:%p dataLength:%u", length, data, dataLength);
	RETURN_HR_IF_NULL(E_POINTER, method);
	RETURN_HR_IF_NULL(E_POINTER, bytesReturned);
	winrt::slim_lock_guard lock(_lock);

	WINTRACE(L"MediaStream::KsMethod method:%s", PKSIDENTIFIER_ToString(method, length).c_str());

	return HRESULT_FROM_WIN32(ERROR_SET_NOT_FOUND);
}

STDMETHODIMP_(NTSTATUS) MediaStream::KsEvent(PKSEVENT evt, ULONG length, LPVOID data, ULONG dataLength, ULONG* bytesReturned)
{
	WINTRACE(L"MediaStream::KsEvent evt:%p len:%u data:%p dataLength:%u", evt, length, data, dataLength);
	RETURN_HR_IF_NULL(E_POINTER, bytesReturned);
	winrt::slim_lock_guard lock(_lock);

	WINTRACE(L"MediaStream::KsEvent event:%s", PKSIDENTIFIER_ToString(evt, length).c_str());
	return HRESULT_FROM_WIN32(ERROR_SET_NOT_FOUND);
}
