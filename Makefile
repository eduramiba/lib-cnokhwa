windows-all: win7-x64 win7-x86 # We want to make sure our library works on Windows 7 too

win7-x64:
	cargo +nightly build -Z build-std=std,panic_abort --target x86_64-win7-windows-msvc --release

win7-x86:
	cargo +nightly build -Z build-std=std,panic_abort --target i686-win7-windows-msvc --release

linux-all: linux-x64 linux-x86

linux-x64:
	cargo build --release

linux-x86:
	cargo build --release --target=i686-unknown-linux-gnu