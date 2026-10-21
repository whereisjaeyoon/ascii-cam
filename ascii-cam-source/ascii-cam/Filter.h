#pragma once
#include <windows.h>
#include <mfidl.h>
#include <cstdint>
#include <memory>
#include "AsciiEngine.h"

enum class FilterOption : uint8_t { Passthrough = 0, ASCII = 1, Edge = 2, Grayscale = 3 };
struct GpuGrayscale;

class Filter
{
public:
    Filter();
    ~Filter();
    HRESULT Initialize(UINT width, UINT height, FilterOption option, IMFMediaType* inputType = nullptr);
    HRESULT Process(IMFSample* sample);
    void Shutdown();
private:
    GUID _inputFormat = GUID_NULL;
    UINT _stride = 0;
    UINT _width = 0;
    UINT _height = 0;
    FilterOption _option = FilterOption::Passthrough;
    AsciiEngine _ascii;
    std::unique_ptr<GpuGrayscale> _grayscale;
};
