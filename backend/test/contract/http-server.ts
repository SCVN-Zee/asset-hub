import { createServer, type IncomingMessage, type Server } from 'node:http'
import type { AddressInfo } from 'node:net'
import { Readable } from 'node:stream'

export type FakeHandler = (req: Request) => Promise<Response>

/**
 * One local HTTP server that stands in for every outside host the Worker calls. Miniflare sends all
 * outbound traffic here (see `outboundService` in the harness) and the `Host` header says which
 * fake answers. A real socket lets a fake drop a connection and see a client abort, which a
 * function-style `outboundService` cannot do.
 *
 * A handler that throws drops the connection, as a lost network would.
 */
export class FakeNetwork {
  private server: Server | null = null
  /** Hosts nobody answers for. A test that reaches one fails in `afterEach`. */
  unknownHosts: string[] = []

  constructor(private readonly handlers: Record<string, FakeHandler>) {}

  async listen(): Promise<number> {
    const server = createServer((req, res) => {
      const host = (req.headers.host ?? '').split(':')[0]
      const handler = this.handlers[host]
      if (!handler) {
        this.unknownHosts.push(`${req.method} ${host}${req.url}`)
        req.resume()
        res.writeHead(502).end()
        return
      }
      const abort = new AbortController()
      res.on('close', () => {
        if (!res.writableFinished) abort.abort()
      })
      const hasBody = req.method !== 'GET' && req.method !== 'HEAD'
      const request = new Request(`https://${host}${req.url}`, {
        method: req.method,
        headers: toHeaders(req),
        body: hasBody ? (Readable.toWeb(req) as ReadableStream<Uint8Array>) : undefined,
        // Node requires this flag for a streamed request body.
        ...(hasBody ? { duplex: 'half' } : {}),
        signal: abort.signal
      } as RequestInit)
      handler(request).then(
        (response) => {
          res.writeHead(response.status, [...response.headers].flat())
          if (!response.body) return void res.end()
          const body = Readable.fromWeb(response.body as never)
          body.on('error', () => req.socket.destroy())
          body.pipe(res)
        },
        () => req.socket.destroy()
      )
    })
    this.server = server
    await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
    return (server.address() as AddressInfo).port
  }

  async close(): Promise<void> {
    const server = this.server
    if (!server) return
    server.closeAllConnections()
    await new Promise<void>((resolve) => server.close(() => resolve()))
    this.server = null
  }
}

function toHeaders(req: IncomingMessage): Headers {
  const headers = new Headers()
  for (const [name, value] of Object.entries(req.headers)) {
    if (value === undefined) continue
    for (const v of Array.isArray(value) ? value : [value]) headers.append(name, v)
  }
  return headers
}
