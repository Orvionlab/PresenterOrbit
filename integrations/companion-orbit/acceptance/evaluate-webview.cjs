const fs = require("node:fs");
(async () => {
  const pages = await (await fetch("http://127.0.0.1:5192/json/list")).json();
  const page = pages.find((item) => item.url === "http://127.0.0.1:5173/");
  if (!page) throw new Error("Main test WebView is unavailable.");
  const socket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    socket.onopen = resolve;
    socket.onerror = reject;
  });
  const result = new Promise((resolve, reject) => {
    socket.onmessage = (event) => {
      const message = JSON.parse(event.data);
      if (message.id === 1) {
        socket.close();
        if (message.error || message.result.exceptionDetails) reject(message.error || message.result.exceptionDetails);
        else resolve(message.result.result.value);
      }
    };
  });
  socket.send(
    JSON.stringify({
      id: 1,
      method: "Runtime.evaluate",
      params: {
        expression: fs.readFileSync(process.argv[2], "utf8"),
        awaitPromise: true,
        returnByValue: true,
      },
    }),
  );
  console.log(JSON.stringify(await result, null, 2));
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
