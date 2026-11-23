#ifdef NDEBUG
#undef NDEBUG
#endif
// Compile the production selector into this isolated API test executable. It
// creates shell items, but neither registers a handler nor operates Explorer UI.
#include "../src/commands.cpp"
#include <shlobj.h>
#include <cassert>
#include <fstream>
#include <map>

ComPtr<IShellItemArray> shell_item(const std::filesystem::path& path) {
    ComPtr<IShellItem> item;
    assert(SUCCEEDED(SHCreateItemFromParsingName(path.c_str(), nullptr, IID_PPV_ARGS(&item))));
    ComPtr<IShellItemArray> array;
    assert(SUCCEEDED(SHCreateShellItemArrayFromShellItem(item.Get(), IID_PPV_ARGS(&array))));
    return array;
}

ComPtr<IShellItemArray> shell_items(const std::vector<std::filesystem::path>& paths) {
    std::vector<PCIDLIST_ABSOLUTE> ids;
    for (const auto& path : paths) {
        PIDLIST_ABSOLUTE id{};
        assert(SUCCEEDED(SHParseDisplayName(path.c_str(), nullptr, &id, 0, nullptr)));
        ids.push_back(id);
    }
    ComPtr<IShellItemArray> array;
    const HRESULT result = SHCreateShellItemArrayFromIDLists(static_cast<UINT>(ids.size()), ids.data(), &array);
    for (const auto id : ids) { CoTaskMemFree(const_cast<PIDLIST_ABSOLUTE>(id)); }
    assert(SUCCEEDED(result));
    return array;
}

void test_file_menu_ownership() {
    const std::wstring app = L"C:\\测试 程序\\qing-zip.exe";
    const std::wstring verb = L"Software\\Classes\\*\\shell\\qing-zip-compress-files";
    const std::wstring clsid = L"{B1031E9C-F2AA-4658-8932-E2596415A942}";
    const std::wstring server = L"Software\\Classes\\CLSID\\" + clsid;
    const auto command = std::make_pair(verb + L"\\command", std::wstring(L"DelegateExecute"));
    std::map<std::pair<std::wstring, std::wstring>, std::wstring> values = {
        {{verb, L"QingZipExecutable"}, app},
        {{server, L"QingZipExecutable"}, app},
        {{server + L"\\LocalServer32", L""}, L"\"C:\\测试 程序\\qing-zip.exe\" --shell-compress-server"},
        {command, clsid},
    };
    const auto matches = [&](const wchar_t* key, const wchar_t* name, const std::wstring& expected) {
        const auto where = values.find({key, name ? name : L""});
        return where != values.end() && where->second == expected;
    };
    assert(file_compression_enabled(app, matches));
    assert(!file_compression_enabled(L"C:\\另一个目录\\qing-zip.exe", matches));
    values[command] = L"{00000000-0000-0000-0000-000000000000}";
    assert(!file_compression_enabled(app, matches));
    values[command] = clsid;
    values.erase({server + L"\\LocalServer32", L""});
    assert(!file_compression_enabled(app, matches));
}

int wmain() {
    assert(SUCCEEDED(CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED)));
    const auto root = std::filesystem::temp_directory_path() / (L"qing-shell-selection-" + std::to_wstring(GetCurrentProcessId()));
    assert(!std::filesystem::exists(root));
    assert(std::filesystem::create_directory(root));
    const auto folder = root / L"中文 folder", archive = root / L"sample.zip", ordinary = root / L"notes.txt";
    assert(std::filesystem::create_directory(folder));
    { std::ofstream file(archive, std::ios::binary); file << "PK\x05\x06"; }
    { std::ofstream file(ordinary, std::ios::binary); file << "text"; }
    {
        auto zip = shell_item(archive), text = shell_item(ordinary), directory = shell_item(folder);
        std::vector<std::wstring> found;
        assert(selection(zip.Get(), qing::Action::SmartExtract, &found) == S_OK);
        assert(found.size() == 1);
        // The shell can expand an 8.3 component in the runner's TEMP path.
        assert(std::filesystem::equivalent(found.front(), archive));
        assert(selection(zip.Get(), qing::Action::ExtractHere, nullptr) == S_OK);
        assert(selection(zip.Get(), qing::Action::Open, nullptr) == S_OK);
        assert(FAILED(selection(zip.Get(), qing::Action::CompressFolder, nullptr)));
        assert(selection(directory.Get(), qing::Action::CompressFolder, nullptr) == S_OK);
        assert(FAILED(selection(directory.Get(), qing::Action::CompressFiles, nullptr)));
        assert(FAILED(selection(directory.Get(), qing::Action::Open, nullptr)));
        assert(FAILED(selection(directory.Get(), qing::Action::SmartExtract, nullptr)));
        assert(selection(text.Get(), qing::Action::Open, nullptr) == S_OK);
        assert(FAILED(selection(text.Get(), qing::Action::SmartExtract, nullptr)));
        assert(FAILED(selection(nullptr, qing::Action::Open, nullptr)));
        assert(selection(text.Get(), qing::Action::CompressFiles, nullptr) == S_OK);
        auto mixed = shell_items({ordinary, folder});
        assert(FAILED(selection(mixed.Get(), qing::Action::CompressFiles, nullptr)));
        assert(FAILED(selection(mixed.Get(), qing::Action::CompressFolder, nullptr)));
    }
    std::vector<std::filesystem::path> files = {archive, ordinary};
    while (files.size() < 129) {
        files.push_back(root / (L"中文 file " + std::to_wstring(files.size()) + L".txt"));
        std::ofstream file(files.back(), std::ios::binary);
        assert(file.good());
    }
    {
        assert(FAILED(selection(shell_items(files).Get(), qing::Action::CompressFiles, nullptr)));
        const std::vector first_128(files.begin(), files.end() - 1);
        auto selection_128 = shell_items(first_128);
        std::vector<std::wstring> forwarded;
        assert(selection(selection_128.Get(), qing::Action::CompressFiles, &forwarded) == S_OK);
        assert(forwarded.size() == 128);
        for (size_t index = 0; index < forwarded.size(); ++index) {
            assert(std::filesystem::equivalent(forwarded[index], files[index]));
        }
    }
    test_file_menu_ownership();
    // Exact test-owned children only; no recursive cleanup of arbitrary paths.
    for (const auto& file : files) { assert(std::filesystem::remove(file)); }
    assert(std::filesystem::remove(folder)); assert(std::filesystem::remove(root));
    CoUninitialize(); return 0;
}
