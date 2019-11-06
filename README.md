
## Prepare rust

```
curl https://sh.rustup.rs -sSf | sh -s -- -y --no-modify-path
rustup target add x86_64-unknown-linux-musl

git clone --depth 1 git://git.musl-libc.org/musl /tmp/musl && \
    cd /tmp/musl && \
    ./configure && \
    make && \
    make install && \
    ln -nfs /usr/local/musl/bin/musl-gcc /usr/local/bin/ && \
    cd / && \
    rm -rf /tmp/musl
```

## Build

```
cargo build --release --features "static" --target x86_64-unknown-linux-musl
strip -s target/x86_64-unknown-linux-musl/release/eit-stream
```
