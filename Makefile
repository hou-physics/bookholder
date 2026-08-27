.PHONY: setup build test dev install clean

test:
	cargo test

dev:
	cd app && ui/node_modules/.bin/tauri dev

build:
	cd app/ui && npm install
	cd app && ui/node_modules/.bin/tauri build

install: build
	rm -rf /Applications/Bookholder.app
	cp -R target/release/bundle/macos/Bookholder.app /Applications/
	open /Applications/Bookholder.app

setup: test install
	@echo "✅ Bookholder 已安装并启动。开机自启请在应用设置页勾选。"

clean:
	cargo clean && rm -rf app/ui/node_modules app/ui/dist
