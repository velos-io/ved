const http = require('http');

const port = process.env.PORT || 8080;
const lambdaUrl = process.env.LAMBDA_URL || 'http://lambda-emulator:8080/2015-03-31/functions/function/invocations';

const server = http.createServer((req, res) => {
  if (req.url === '/health' && req.method === 'GET') {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ status: 'ok' }));
    return;
  }

  if (req.url === '/smoke' && req.method === 'GET') {
    const url = new URL(lambdaUrl);
    const payload = JSON.stringify({
      path: '/smoke',
      httpMethod: 'GET',
      headers: req.headers
    });

    const lambdaReq = http.request({
      hostname: url.hostname,
      port: url.port || 8080,
      path: url.pathname,
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Content-Length': Buffer.byteLength(payload)
      },
      timeout: 5000
    }, (lambdaRes) => {
      let data = '';
      lambdaRes.on('data', chunk => { data += chunk; });
      lambdaRes.on('end', () => {
        try {
          const parsed = JSON.parse(data);
          const statusCode = parsed.statusCode || lambdaRes.statusCode || 200;
          const headers = parsed.headers || { 'Content-Type': 'application/json' };
          res.writeHead(statusCode, headers);
          const body = typeof parsed.body === 'string' ? parsed.body : JSON.stringify(parsed.body || { status: 'ok' });
          res.end(body);
        } catch (_) {
          res.writeHead(lambdaRes.statusCode || 200, { 'Content-Type': 'application/json' });
          res.end(data);
        }
      });
    });

    lambdaReq.on('error', (err) => {
      res.writeHead(502, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'lambda unreachable', message: err.message }));
    });

    lambdaReq.on('timeout', () => {
      lambdaReq.destroy();
      res.writeHead(504, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'lambda timeout' }));
    });

    lambdaReq.write(payload);
    lambdaReq.end();
    return;
  }

  res.writeHead(404, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ error: 'not found' }));
});

server.listen(port, () => {
  console.log(`API Gateway listening on port ${port}, routing /smoke to ${lambdaUrl}`);
});
