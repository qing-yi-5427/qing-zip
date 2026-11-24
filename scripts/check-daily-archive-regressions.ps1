# Execute existing ordinary-file fixtures individually so a renamed test cannot silently pass.
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$manifest = Join-Path $PSScriptRoot '../obz-rs/Cargo.toml'
$suites = @(
    @{
        Target = @('--test', 'engine_integration')
        Cases = @(
            'engine_zip_and_7z_encryption_roundtrip'
            'extract_small_zip_into_existing_long_directory_preserves_files_and_conflicts'
            'zipcrypto_non_ascii_password_fails_with_an_actionable_error'
            'engine_compound_tar_one_step_and_metadata'
            'engine_legacy_codepage_override'
            'engine_update_preserves_original_on_failure'
            'engine_concurrent_updates_preserve_both_additions'
        )
    },
    @{
        Target = @('--test', 'archive_comment')
        Cases = @(
            'saved_comments_stay_readable_by_legacy_code_page_readers'
        )
    },
    @{
        Target = @('--lib')
        Cases = @(
            'engine::staging_tests::staged_entry_attributes_are_restored_beyond_max_path'
            'engine::nested::tests::nested_zip_writeback_preserves_sibling_and_allows_a_second_save'
            'engine::nested::tests::externally_changed_outer_member_prevents_writeback_and_keeps_edited_copy'
            'engine::nested::tests::two_levels_of_nested_edits_round_trip_to_the_outer_archive'
            'engine::nested::tests::authenticated_outer_zip_keeps_member_encrypted_after_nested_writeback'
            'engine::nested::tests::nested_recovery_requires_changed_bytes_committed_to_the_owned_working_copy'
            'engine::nested::tests::nested_recovery_excludes_a_zip_with_an_unrelated_encrypted_member'
            'engine::update_modes_tests::update_modes_real_zip_and_solid_7z_follow_official_action_matrix'
        )
    }
)
foreach ($suite in $suites) {
    $target = $suite.Target
    foreach ($case in $suite.Cases) {
        $result = cargo test --locked --manifest-path $manifest @target $case -- --exact 2>&1
        $code = $LASTEXITCODE
        $summary = $result -match '^test result: ok\. 1 passed; 0 failed;'
        if ($code -ne 0 -or -not $summary) {
            $result | Out-String | Write-Output
            throw "Expected exactly one passing test for $case"
        }
        Write-Output "$case — $summary"
    }
}
