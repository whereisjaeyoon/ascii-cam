#include "pch.h"
#include "FilterSubscriber.h"
#include <sddl.h>

#pragma comment(lib, "advapi32.lib")

namespace
{
    constexpr char kMapName[] = "Global\\AsciiCam";
    constexpr char kPublishedName[] = "Global\\AsciiCamPublished";
    constexpr char kConsumedName[] = "Global\\AsciiCamConsumed";
    constexpr ULONGLONG kRetryIntervalMs = 1000;
    constexpr uint32_t kMaxFilterValue = 3;

    // The producer may run under a different account than the Frame Server hosting this DLL,
    // so the objects are opened to everyone (SDDL "WD" = World).
    constexpr char kOpenAccessSddl[] = "D:(A;;GA;;;WD)";
}

FilterSubscriber::~FilterSubscriber()
{
    Close();
}

void FilterSubscriber::Close()
{
    if (_state) { UnmapViewOfFile(const_cast<SharedFilterState*>(_state)); _state = nullptr; }
    if (_map) { CloseHandle(_map); _map = nullptr; }
    if (_published) { CloseHandle(_published); _published = nullptr; }
    if (_consumed) { CloseHandle(_consumed); _consumed = nullptr; }
}

bool FilterSubscriber::EnsureOpen()
{
    if (_state)
        return true;

    // Creating can fail persistently (e.g. foreign object with the same name); don't spin on it.
    ULONGLONG now = GetTickCount64();
    if (_lastOpenAttempt != 0 && now - _lastOpenAttempt < kRetryIntervalMs)
        return false;
    _lastOpenAttempt = now;

    PSECURITY_DESCRIPTOR descriptor = nullptr;
    if (!ConvertStringSecurityDescriptorToSecurityDescriptorA(
            kOpenAccessSddl, SDDL_REVISION_1, &descriptor, nullptr))
    {
        WINTRACE(L"FilterSubscriber: security descriptor failed: %lu", GetLastError());
        return false;
    }

    SECURITY_ATTRIBUTES attributes = {};
    attributes.nLength = sizeof(attributes);
    attributes.lpSecurityDescriptor = descriptor;
    attributes.bInheritHandle = FALSE;

    bool opened = false;
    bool created = false;

    _map = CreateFileMappingA(INVALID_HANDLE_VALUE, &attributes, PAGE_READWRITE,
                              0, sizeof(SharedFilterState), kMapName);
    if (_map)
    {
        created = GetLastError() != ERROR_ALREADY_EXISTS;

        _state = static_cast<volatile SharedFilterState*>(
            MapViewOfFile(_map, FILE_MAP_ALL_ACCESS, 0, 0, sizeof(SharedFilterState)));
    }

    if (_state)
    {
        // Auto-reset events. Consumed starts signaled; the initial state only applies when the
        // event is created here, an existing one keeps its current state.
        _published = CreateEventA(&attributes, FALSE, FALSE, kPublishedName);
        _consumed = CreateEventA(&attributes, FALSE, TRUE, kConsumedName);
        opened = _published && _consumed;
    }

    LocalFree(descriptor);

    if (opened)
    {
        if (created)
        {
            // Initialize the payload first and publish `magic` last, so the producer never
            // sees a valid magic over uninitialized fields.
            _state->version = kSharedFilterVersion;
            _state->value = static_cast<uint32_t>(FilterOption::Passthrough);
            _state->sequence = 0;
            MemoryBarrier();
            _state->magic = kSharedFilterMagic;
        }
        else if (_state->magic != kSharedFilterMagic || _state->version != kSharedFilterVersion)
        {
            WINTRACE(L"FilterSubscriber: existing shared memory has wrong magic/version");
            opened = false;
        }
    }

    if (!opened)
    {
        Close();
        return false;
    }

    return true;
}

bool FilterSubscriber::TryGetUpdate(FilterOption& option)
{
    if (!EnsureOpen())
        return false;

    if (WaitForSingleObject(_published, 0) != WAIT_OBJECT_0)
        return false;

    // The producer wrote the payload before signaling; fence before reading it.
    MemoryBarrier();
    uint32_t magic = _state->magic;
    uint32_t version = _state->version;
    uint32_t value = _state->value;

    if (magic != kSharedFilterMagic || version != kSharedFilterVersion || value > kMaxFilterValue)
    {
        WINTRACE(L"FilterSubscriber: ignoring invalid publish (magic=0x%08X version=%u value=%u)",
                 magic, version, value);
        // Still acknowledge, otherwise the producer would wait forever.
        NotifyConsumed();
        return false;
    }

    option = static_cast<FilterOption>(value);
    return true;
}

void FilterSubscriber::NotifyConsumed()
{
    if (_consumed)
        SetEvent(_consumed);
}
