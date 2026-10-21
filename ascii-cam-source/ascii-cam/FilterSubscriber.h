#pragma once

#include <windows.h>
#include <cstdint>
#include "Filter.h"

// Consumer side of the AsciiCam shared-memory interface (the producer is the Rust controller).
// The consumer creates and initializes every object; the producer only opens them.
// The DLL runs in the Frame Server service (session 0); Global\ makes the names visible to the
// producer in the user session.
//
//   Mapping : Global\AsciiCam           SharedFilterState
//   Event   : Global\AsciiCamPublished  auto-reset, producer -> consumer: "a new value was written"
//   Event   : Global\AsciiCamConsumed   auto-reset, consumer -> producer: "the value was applied"
//                                (created signaled so the first publish does not block)
//
// FilterOption values: 0 = Passthrough, 1 = ASCII, 2 = Edge, 3 = Grayscale.
struct SharedFilterState
{
    uint32_t magic;
    uint32_t version;
    uint32_t value;
    uint32_t sequence;
};

constexpr uint32_t kSharedFilterMagic = 0x41534349;
constexpr uint32_t kSharedFilterVersion = 1;

class FilterSubscriber
{
public:
    FilterSubscriber() = default;
    ~FilterSubscriber();

    FilterSubscriber(const FilterSubscriber&) = delete;
    FilterSubscriber& operator=(const FilterSubscriber&) = delete;

    // Non-blocking, intended to be polled once per frame. Returns true and fills `option` when a
    // valid value was published. The caller must apply it and then call NotifyConsumed();
    // until then the producer is blocked from publishing again. Invalid publishes are
    // acknowledged internally and reported as false.
    bool TryGetUpdate(FilterOption& option);

    // Tells the producer that the value returned by TryGetUpdate has been handled.
    void NotifyConsumed();

    void Close();

private:
    bool EnsureOpen();

    HANDLE _map = nullptr;
    HANDLE _published = nullptr;
    HANDLE _consumed = nullptr;
    volatile SharedFilterState* _state = nullptr;
    ULONGLONG _lastOpenAttempt = 0;
};
