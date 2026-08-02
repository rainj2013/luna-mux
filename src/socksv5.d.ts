declare module 'socksv5' {
  import type { Duplex } from 'node:stream'
  import type { AddressInfo } from 'node:net'

  interface SocksRequestInfo {
    srcAddr: string
    srcPort: number
    dstAddr: string
    dstPort: number
  }

  export interface SocksServer {
    listen(port: number, host: string, callback: () => void): this
    close(callback?: (error?: Error) => void): this
    address(): AddressInfo | string | null
    on(event: 'error', listener: (error: Error) => void): this
    once(event: 'error', listener: (error: Error) => void): this
    removeListener(event: 'error', listener: (error: Error) => void): this
    useAuth(auth: unknown): this
  }

  export function createServer(handler: (
    info: SocksRequestInfo,
    accept: (intercept?: boolean) => Duplex | undefined,
    deny: () => void
  ) => void): SocksServer

  export const auth: { None(): unknown }
  const socks: { createServer: typeof createServer; auth: typeof auth }
  export default socks
}
