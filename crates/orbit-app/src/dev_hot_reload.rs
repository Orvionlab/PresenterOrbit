pub fn arguments(json: &str) -> Result<Vec<String>, String> {
    serde_json::from_str(json).map_err(|error| format!("Invalid PRESENTER_ORBIT_HOT_ARGS: {error}"))
}

// dx 0.7.10's Windows x64 external-symbol trampoline clobbers RAX, but __chkstk
// receives the allocation size in RAX. Keep the probe in the patch's own objects
// so it never goes through that trampoline. Normal builds use the CRT probe.
#[cfg(all(feature = "dev-hot-reload", windows, target_arch = "x86_64"))]
std::arch::global_asm!(
    ".text",
    ".globl __chkstk",
    ".def __chkstk; .scl 2; .type 32; .endef",
    ".p2align 4",
    "__chkstk:",
    "push rcx",
    "push rax",
    "lea rcx, [rsp + 24]",
    "2:",
    "cmp rax, 4096",
    "jb 3f",
    "sub rcx, 4096",
    "test qword ptr [rcx], rax",
    "sub rax, 4096",
    "jmp 2b",
    "3:",
    "sub rcx, rax",
    "test qword ptr [rcx], rax",
    "pop rax",
    "pop rcx",
    "ret",
);
