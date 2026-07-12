const http = require("http");

http
  .createServer((_request, response) => {
    response.writeHead(200, { "content-type": "text/plain; charset=utf-8" });
    response.end("Hello from a Yarn Classic Stackstead.\n");
  })
  .listen(3000, "0.0.0.0");
