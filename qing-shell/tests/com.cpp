#ifdef NDEBUG
#undef NDEBUG
#endif
#include <windows.h>
#include <shobjidl.h>
#include <wrl/client.h>
#include <cassert>
#include <string>

using Microsoft::WRL::ComPtr;
int wmain(int argc, wchar_t** argv) {
    assert(argc == 2);
    assert(SUCCEEDED(CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED)));
    const HMODULE module = LoadLibraryExW(argv[1], nullptr, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32);
    assert(module);
    const auto get_class = reinterpret_cast<HRESULT(WINAPI*)(REFCLSID, REFIID, void**)>(GetProcAddress(module, "DllGetClassObject"));
    const auto can_unload = reinterpret_cast<HRESULT(WINAPI*)()>(GetProcAddress(module, "DllCanUnloadNow"));
    assert(get_class && can_unload && can_unload() == S_OK);
    const wchar_t* ids[] = {L"{9A406E20-1F79-49F3-94F9-179D18761D4A}", L"{A65CB9C7-E692-4D0B-A5A1-4F19530EE312}",
        L"{8148C339-AF9A-4E1E-8BE9-469F28DA1B75}", L"{0C0C73E7-2D76-4823-AB70-F9BE332B1360}"};
    const wchar_t* titles[] = {L"用 qing-zip 打开", L"用 qing-zip 压缩…", L"智能解压", L"解压到此处"};
    for (size_t index = 0; index < 4; ++index) {
        CLSID id{}; assert(SUCCEEDED(CLSIDFromString(ids[index], &id)));
        ComPtr<IClassFactory> factory;
        assert(SUCCEEDED(get_class(id, IID_PPV_ARGS(&factory))));
        assert(can_unload() == S_FALSE);
        ComPtr<IExplorerCommand> command;
        assert(SUCCEEDED(factory->CreateInstance(nullptr, IID_PPV_ARGS(&command))));
        GUID actual{}; assert(command->GetCanonicalName(&actual) == S_OK && actual == id);
        PWSTR title{}; assert(command->GetTitle(nullptr, &title) == S_OK && std::wstring(title) == titles[index]); CoTaskMemFree(title);
        PWSTR tooltip{}; assert(command->GetToolTip(nullptr, &tooltip) == S_OK && tooltip && tooltip[0]); CoTaskMemFree(tooltip);
        PWSTR icon{}; assert(command->GetIcon(nullptr, &icon) == S_OK && std::wstring(icon).ends_with(L"qing-zip.exe\",0")); CoTaskMemFree(icon);
        EXPCMDSTATE state{};
        assert(command->GetState(nullptr, FALSE, &state) == E_PENDING && state == ECS_DISABLED);
        assert(command->GetState(nullptr, TRUE, &state) == S_OK && state == ECS_HIDDEN);
        EXPCMDFLAGS flags{}; assert(command->GetFlags(&flags) == S_OK && flags == ECF_DEFAULT);
        ComPtr<IEnumExplorerCommand> children; assert(command->EnumSubCommands(&children) == E_NOTIMPL && !children);
        // No registry registration and no invocation of the user's application.
        assert(FAILED(command->Invoke(nullptr, nullptr)));
        assert(command->GetTitle(nullptr, nullptr) == E_POINTER);
        assert(factory->LockServer(TRUE) == S_OK); assert(factory->LockServer(FALSE) == S_OK);
    }
    void* unknown{};
    assert(get_class(CLSID_NULL, IID_IUnknown, &unknown) == CLASS_E_CLASSNOTAVAILABLE && !unknown);
    assert(can_unload() == S_OK);
    FreeLibrary(module); CoUninitialize();
    return 0;
}
