#pragma once
#include <windows.h>
#include <mfidl.h>
#include <cstdint>
#include <memory>
#include "AsciiEngine.h"
#include "GrayscaleEngine.h"

enum class FilterOption : uint8_t { Passthrough = 0, ASCII = 1, Edge = 2, Grayscale = 3 };
struct GpuGrayscale;

class Filter
{
public:
    Filter();
    ~Filter();

    HRESULT Initialize(UINT width, UINT height, FilterOption option);

    // Switches the active filter at runtime, lazily creating the engine it needs.
    // On failure the previous filter stays active. Call from the thread that calls Process.
    HRESULT SetOption(FilterOption option);
    HRESULT Process(IMFSample* sample);
    void Shutdown();
private:
    HRESULT EnsureEngine(FilterOption option);

    UINT _width = 0;
    UINT _height = 0;
    FilterOption _option = FilterOption::Passthrough;

    bool _asciiReady = false;
    bool _grayscaleReady = false;

    AsciiEngine _ascii;
    GrayscaleEngine _grayscale;
};
