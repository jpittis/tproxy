This is still a WIP, but the rough idea is to enable a flow like this:

1. Tell the proxy to forward requests from one port to another.

```
cargo run -- -l 127.0.0.1:4444 -u 127.0.0.1:5555
```

2. Setup a backend server.

```
nc -lp 5555
```

3. Run a request through the proxy.

```
nc 127.0.0.1 4444
```

4. Visit the debug server in your browser.

```
firefox 127.0.0.1:2222
```

5. Terminate the active connection using the debug UI.
