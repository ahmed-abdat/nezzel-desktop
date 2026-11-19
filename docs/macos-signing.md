# macOS signing

Until we have an Apple Developer ID, macOS releases are signed with one fixed self-signed
code-signing identity, "Nezzel Self-Signed" (RSA 2048, valid to 2046, key usage digital signature,
extended key usage code signing). `publish.yml` imports it from the secrets `MACOS_SELF_SIGN_P12`
(base64 p12) and `MACOS_SELF_SIGN_PASSWORD` into a temporary keychain and signs the app, its
sidecars and the yt-dlp/gallery-dl onedirs with it (no notarization). Because the identity never
changes, the designated requirement stays `identifier "dev.ahmed.nezzel" and certificate root =
H"<cert SHA-1>"` across releases, so macOS keeps the Automation (Chrome) grant and the keychain
item access after each update; ad-hoc builds pinned the cdhash, which changed every build.
Gatekeeper still treats the app like an unsigned one (first open needs right-click → Open or
Privacy & Security → Open Anyway). The private key lives only on the owner's Mac in
`~/.config/nezzel-signing/` (`cert.p12`, `cert.p12.password`, `cert.pem`; mode 600) and in the two
GitHub secrets; never commit it. Rotating it (new certificate, or moving to a Developer ID) changes
the designated requirement once more: every user gets one more keychain prompt and has to allow
Automation again after that update. The bundled yt-dlp and gallery-dl onedirs are signed with
`scripts/tools-entitlements-self-signed.plist`, which turns library validation off: without a
Team ID, macOS refuses to let their bootloader load their own Python ("mapping process and mapped
file (non-platform) have different Team IDs"). Neither secret set means ad-hoc signing; `APPLE_CERTIFICATE`
set means the Developer ID path, which takes precedence. The Tauri updater is unaffected: it checks
its own minisign signature over `Nezzel.app.tar.gz`, not the Apple code signature.
