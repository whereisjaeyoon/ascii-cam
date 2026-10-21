#pragma once

#include <windows.h>
#include <d3d11.h>
#include <wil/com.h>
#include <wil/result.h>
#include <cstdint>

class GrayscaleEngine
{
public:
    GrayscaleEngine() = default;
    ~GrayscaleEngine();

    HRESULT Initialize(UINT width, UINT height);

    HRESULT Process(const BYTE* yPlane, LONG pitch);

    void Shutdown();

private:
    UINT _width = 0;
    UINT _height = 0;

    wil::com_ptr_nothrow<ID3D11Device> _device;
    wil::com_ptr_nothrow<ID3D11DeviceContext> _context;
    wil::com_ptr_nothrow<ID3D11Texture2D> _inputTexture;
    wil::com_ptr_nothrow<ID3D11ShaderResourceView> _inputSRV;
    wil::com_ptr_nothrow<ID3D11Texture2D> _outputTexture;
    wil::com_ptr_nothrow<ID3D11UnorderedAccessView> _outputUAV;
    wil::com_ptr_nothrow<ID3D11Texture2D> _stagingTexture;
    wil::com_ptr_nothrow<ID3D11ComputeShader> _computeShader;

    static constexpr UINT THREAD_X = 8;
    static constexpr UINT THREAD_Y = 8;
};