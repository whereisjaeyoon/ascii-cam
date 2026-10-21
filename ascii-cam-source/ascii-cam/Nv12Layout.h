#pragma once
#include <cstdint>
#include <limits>

// NV12 has height Y rows followed by height/2 interleaved UV rows, each
// containing width bytes. Pitch can be larger than width; 1.5 is NOT a stride.
struct Nv12Layout
{
    std::uint32_t rows = 0;
    std::uint32_t packedBytes = 0;
    std::uint32_t footprint = 0;

    static bool TryCreate(std::uint32_t width, std::uint32_t height,
        std::uint32_t pitch, Nv12Layout& result)
    {
        result = {};
        if (!width || !height || width % 2 || height % 2 || pitch < width ||
            pitch > static_cast<std::uint32_t>((std::numeric_limits<std::int32_t>::max)()))
            return false;
        const std::uint64_t rows = static_cast<std::uint64_t>(height) + height / 2;
        const std::uint64_t packed = rows * width;
        // The last row needs only its active bytes, not trailing pitch padding.
        const std::uint64_t footprint = (rows - 1) * pitch + width;
        const auto maximum = (std::numeric_limits<std::uint32_t>::max)();
        if (rows > maximum || packed > maximum || footprint > maximum) return false;
        result = { static_cast<std::uint32_t>(rows), static_cast<std::uint32_t>(packed),
            static_cast<std::uint32_t>(footprint) };
        return true;
    }
};
