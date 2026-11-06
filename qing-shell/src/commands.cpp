#include "launch.h"
#include <shobjidl.h>
#include <shlwapi.h>
#include <wrl/client.h>
#include <atomic>
#include <new>

using Microsoft::WRL::ComPtr;
namespace {
HMODULE module_handle{};
std::atomic<long> objects{0}, server_locks{0};
// Stable product-owned CLSIDs; the sparse manifest must match these exactly.
const CLSID open_id = {0x9a406e20,0x1f79,0x49f3,{0x94,0xf9,0x17,0x9d,0x18,0x76,0x1d,0x4a}};
const CLSID compress_id = {0xa65cb9c7,0xe692,0x4d0b,{0xa5,0xa1,0x4f,0x19,0x53,0x0e,0xe3,0x12}};

const CLSID smart_id = {0x8148c339,0xaf9a,0x4e1e,{0x8b,0xe9,0x46,0x9f,0x28,0xda,0x1b,0x75}};
const CLSID here_id = {0x0c0c73e7,0x2d76,0x4823,{0xab,0x70,0xf9,0xbe,0x33,0x2b,0x13,0x60}};
const CLSID& action_id(qing::Action action) {
    switch (action) {
    case qing::Action::Compress: return compress_id;
    case qing::Action::SmartExtract: return smart_id;
    case qing::Action::ExtractHere: return here_id;
    default: return open_id;
    }
}
struct TaskString {
    PWSTR value{};
    ~TaskString() { CoTaskMemFree(value); }
};
bool setting_enabled(bool compress, const std::wstring& executable) {
    const wchar_t* key = compress
        ? L"Software\\Classes\\Directory\\shell\\qing-zip-compress\\command"
        : L"Software\\Classes\\*\\shell\\qing-zip\\command";
    DWORD length = 0;
    if (RegGetValueW(HKEY_CURRENT_USER, key, nullptr, RRF_RT_REG_SZ, nullptr, nullptr, &length)
        != ERROR_SUCCESS || length < sizeof(wchar_t) || length > 65536) { return false; }
    std::vector<wchar_t> value(length / sizeof(wchar_t) + 1, 0);
    if (RegGetValueW(HKEY_CURRENT_USER, key, nullptr, RRF_RT_REG_SZ, nullptr, value.data(), &length)
        != ERROR_SUCCESS) { return false; }
    const auto expected = qing::quote(executable)
        + (compress ? L" --compress-folder \"%1\"" : L" \"%1\"");
    return CompareStringOrdinal(value.data(), -1, expected.c_str(), -1, TRUE) == CSTR_EQUAL;
}
HRESULT selection(IShellItemArray* items, qing::Action action, std::vector<std::wstring>* paths) {
    const bool compress = action == qing::Action::Compress;
    if (!items) { return E_INVALIDARG; }
    DWORD count = 0;
    HRESULT hr = items->GetCount(&count);
    if (FAILED(hr)) { return hr; }
    if (count == 0 || count > 128) { return E_INVALIDARG; }
    for (DWORD index = 0; index < count; ++index) {
        ComPtr<IShellItem> item;
        hr = items->GetItemAt(index, &item);
        if (FAILED(hr)) { return hr; }
        SFGAOF attributes{};
        hr = item->GetAttributes(SFGAO_FILESYSTEM, &attributes);
        if (FAILED(hr)) { return hr; }
        if ((attributes & SFGAO_FILESYSTEM) == 0) { return E_INVALIDARG; }
        TaskString path;
        hr = item->GetDisplayName(SIGDN_FILESYSPATH, &path.value);
        if (FAILED(hr)) { return hr; }
        if (!path.value || !qing::supported_path(path.value)) { return E_INVALIDARG; }
        // ZIP files can advertise SFGAO_FOLDER through Explorer's compressed
        // folder provider. Use the actual filesystem kind, not that shell flag.
        const DWORD file_attributes = GetFileAttributesW(path.value);
        if (file_attributes == INVALID_FILE_ATTRIBUTES || (file_attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0
            || ((file_attributes & FILE_ATTRIBUTE_DIRECTORY) != 0) != compress) {
            return E_INVALIDARG;
        }
        if ((action == qing::Action::SmartExtract || action == qing::Action::ExtractHere)
            && !qing::archive_path(path.value)) { return E_INVALIDARG; }
        if (paths) { paths->emplace_back(path.value); }
    }
    return S_OK;
}
class Command final : public IExplorerCommand {
    std::atomic<ULONG> references_{1};
    qing::Action action_;
public:
    explicit Command(qing::Action action) : action_(action) { ++objects; }
    ~Command() { --objects; }
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void** value) override {
        if (!value) { return E_POINTER; }
        *value = nullptr;
        if (iid == IID_IUnknown || iid == IID_IExplorerCommand) {
            *value = static_cast<IExplorerCommand*>(this); AddRef(); return S_OK;
        }
        return E_NOINTERFACE;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return ++references_; }
    ULONG STDMETHODCALLTYPE Release() override {
        const ULONG result = --references_; if (!result) { delete this; } return result;
    }
    HRESULT STDMETHODCALLTYPE GetTitle(IShellItemArray*, PWSTR* text) override {
        if (!text) { return E_POINTER; }
        const wchar_t* title = L"用 qing-zip 打开";
        switch (action_) {
        case qing::Action::Compress: title = L"用 qing-zip 压缩…"; break;
        case qing::Action::SmartExtract: title = L"智能解压"; break;
        case qing::Action::ExtractHere: title = L"解压到此处"; break;
        default: break;
        }
        return SHStrDupW(title, text);
    }
    HRESULT STDMETHODCALLTYPE GetIcon(IShellItemArray*, PWSTR* text) override {
        if (!text) { return E_POINTER; } *text = nullptr;
        try {
            std::wstring executable; const HRESULT hr = qing::app_path(module_handle, executable);
            if (FAILED(hr)) { return hr; }
            return SHStrDupW((qing::quote(executable) + L",0").c_str(), text);
        } catch (const std::bad_alloc&) { return E_OUTOFMEMORY; } catch (...) { return E_FAIL; }
    }
    HRESULT STDMETHODCALLTYPE GetToolTip(IShellItemArray*, PWSTR* text) override {
        if (!text) { return E_POINTER; }
        const wchar_t* tooltip = L"在 qing-zip 中浏览文件";
        switch (action_) {
        case qing::Action::Compress: tooltip = L"选择格式和保存位置"; break;
        case qing::Action::SmartExtract: tooltip = L"按首级文件夹结构智能选择目标位置"; break;
        case qing::Action::ExtractHere: tooltip = L"解压到压缩包所在文件夹"; break;
        default: break;
        }
        return SHStrDupW(tooltip, text);
    }
    HRESULT STDMETHODCALLTYPE GetCanonicalName(GUID* id) override {
        if (!id) { return E_POINTER; } *id = action_id(action_); return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetState(IShellItemArray* items, BOOL ok_to_be_slow, EXPCMDSTATE* state) override {
        if (!state) { return E_POINTER; } *state = ECS_HIDDEN;
        // Shell attributes may touch remote providers. Ask Explorer to evaluate
        // them on its background callback, never stall the menu's UI thread.
        if (!ok_to_be_slow) { *state = ECS_DISABLED; return E_PENDING; }
        try {
            std::wstring executable;
            if (SUCCEEDED(qing::app_path(module_handle, executable)) && setting_enabled(action_ == qing::Action::Compress, executable)
                && SUCCEEDED(selection(items, action_, nullptr))) { *state = ECS_ENABLED; }
            return S_OK;
        } catch (const std::bad_alloc&) { return E_OUTOFMEMORY; } catch (...) { return E_FAIL; }
    }
    HRESULT STDMETHODCALLTYPE Invoke(IShellItemArray* items, IBindCtx*) override {
        try {
            std::wstring executable; HRESULT hr = qing::app_path(module_handle, executable);
            if (FAILED(hr)) { return hr; }
            if (!setting_enabled(action_ == qing::Action::Compress, executable)) { return HRESULT_FROM_WIN32(ERROR_NOT_FOUND); }
            std::vector<std::wstring> paths;
            hr = selection(items, action_, &paths);
            return FAILED(hr) ? hr : qing::launch(executable, action_, paths);
        } catch (const std::bad_alloc&) { return E_OUTOFMEMORY; } catch (...) { return E_FAIL; }
    }
    HRESULT STDMETHODCALLTYPE GetFlags(EXPCMDFLAGS* flags) override {
        if (!flags) { return E_POINTER; } *flags = ECF_DEFAULT; return S_OK;
    }
    HRESULT STDMETHODCALLTYPE EnumSubCommands(IEnumExplorerCommand** commands) override {
        if (!commands) { return E_POINTER; } *commands = nullptr; return E_NOTIMPL;
    }
};
class Factory final : public IClassFactory {
    std::atomic<ULONG> references_{1};
    qing::Action action_;
public:
    explicit Factory(qing::Action action) : action_(action) { ++objects; }
    ~Factory() { --objects; }
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void** value) override {
        if (!value) { return E_POINTER; } *value = nullptr;
        if (iid == IID_IUnknown || iid == IID_IClassFactory) {
            *value = static_cast<IClassFactory*>(this); AddRef(); return S_OK;
        }
        return E_NOINTERFACE;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return ++references_; }
    ULONG STDMETHODCALLTYPE Release() override {
        const ULONG result = --references_; if (!result) { delete this; } return result;
    }
    HRESULT STDMETHODCALLTYPE CreateInstance(IUnknown* outer, REFIID iid, void** value) override {
        if (!value) { return E_POINTER; } *value = nullptr;
        if (outer) { return CLASS_E_NOAGGREGATION; }
        auto* command = new (std::nothrow) Command(action_);
        if (!command) { return E_OUTOFMEMORY; }
        const HRESULT result = command->QueryInterface(iid, value); command->Release(); return result;
    }
    HRESULT STDMETHODCALLTYPE LockServer(BOOL lock) override {
        if (lock) { ++server_locks; } else { --server_locks; } return S_OK;
    }
};
}
extern "C" BOOL WINAPI DllMain(HINSTANCE module, DWORD reason, LPVOID) {
    // The static CRT requires its thread notifications; don't disable them.
    if (reason == DLL_PROCESS_ATTACH) { module_handle = module; }
    return TRUE;
}
extern "C" HRESULT WINAPI DllCanUnloadNow() {
    return objects.load() == 0 && server_locks.load() == 0 ? S_OK : S_FALSE;
}
extern "C" HRESULT WINAPI DllGetClassObject(REFCLSID clsid, REFIID iid, void** value) {
    if (!value) { return E_POINTER; } *value = nullptr;
    if (clsid != open_id && clsid != compress_id && clsid != smart_id && clsid != here_id) { return CLASS_E_CLASSNOTAVAILABLE; }
    const qing::Action action = clsid == compress_id ? qing::Action::Compress
        : clsid == smart_id ? qing::Action::SmartExtract
        : clsid == here_id ? qing::Action::ExtractHere : qing::Action::Open;
    auto* factory = new (std::nothrow) Factory(action);
    if (!factory) { return E_OUTOFMEMORY; }
    const HRESULT result = factory->QueryInterface(iid, value); factory->Release(); return result;
}
