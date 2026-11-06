#ifdef NDEBUG
#undef NDEBUG
#endif
// Compile the production selector into this isolated API test executable. It
// creates shell items, but neither registers a handler nor operates Explorer UI.
#include "../src/commands.cpp"
#include <cassert>
#include <fstream>

ComPtr<IShellItemArray> shell_item(const std::filesystem::path& path) {
    ComPtr<IShellItem> item;
    assert(SUCCEEDED(SHCreateItemFromParsingName(path.c_str(), nullptr, IID_PPV_ARGS(&item))));
    ComPtr<IShellItemArray> array;
    assert(SUCCEEDED(SHCreateShellItemArrayFromShellItem(item.Get(), IID_PPV_ARGS(&array))));
    return array;
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
        assert(selection(zip.Get(), qing::Action::SmartExtract, &found) == S_OK && found.size() == 1 && found[0] == archive.wstring());
        assert(selection(zip.Get(), qing::Action::ExtractHere, nullptr) == S_OK);
        assert(selection(zip.Get(), qing::Action::Open, nullptr) == S_OK);
        assert(FAILED(selection(zip.Get(), qing::Action::Compress, nullptr)));
        assert(selection(directory.Get(), qing::Action::Compress, nullptr) == S_OK);
        assert(FAILED(selection(directory.Get(), qing::Action::Open, nullptr)));
        assert(FAILED(selection(directory.Get(), qing::Action::SmartExtract, nullptr)));
        assert(selection(text.Get(), qing::Action::Open, nullptr) == S_OK);
        assert(FAILED(selection(text.Get(), qing::Action::SmartExtract, nullptr)));
        assert(FAILED(selection(nullptr, qing::Action::Open, nullptr)));
    }
    // Exact test-owned children only; no recursive cleanup of arbitrary paths.
    assert(std::filesystem::remove(archive)); assert(std::filesystem::remove(ordinary));
    assert(std::filesystem::remove(folder)); assert(std::filesystem::remove(root));
    CoUninitialize(); return 0;
}
