#pragma once

#include <mfapi.h>
#include <mfidl.h>
#include <mfreadwrite.h>
#include <mfobjects.h>
#include <wrl/client.h>
#include <memory>

struct WebcamReadState;

class WebcamCapture
{
public:
    WebcamCapture();
    ~WebcamCapture();
    HRESULT Initialize(UINT width = 1280, UINT height = 720, UINT fps = 30);
    HRESULT GetFrame(IMFSample** sample);

    // GetOutputType removed: keep WebcamCapture interface minimal. Use
    // post-processor Initialize(width, height, FilterOption::ASCII) in
    // MediaStream instead of querying the capture output type here.

    void Shutdown();

private:
    Microsoft::WRL::ComPtr<IMFMediaSource> _source;
    Microsoft::WRL::ComPtr<IMFSourceReader> _reader;
    Microsoft::WRL::ComPtr<IMFSourceReaderCallback> _callback;
    std::shared_ptr<WebcamReadState> _readState;
    UINT _width = 0;
    UINT _height = 0;
};
