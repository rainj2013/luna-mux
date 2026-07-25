import { StringDecoder } from 'node:string_decoder'

type TerminalStream = 'stdout' | 'stderr'

export class TerminalOutputBuffer {
  private readonly decoders: Record<TerminalStream, StringDecoder> = {
    stdout: new StringDecoder('utf8'),
    stderr: new StringDecoder('utf8')
  }
  private chunks: string[] = []
  private bufferedLength = 0
  private timer?: ReturnType<typeof setTimeout>
  private ended = false

  constructor(
    private readonly emit: (data: string) => void,
    private readonly flushDelay = 16,
    private readonly maxBatchLength = 64 * 1024
  ) {}

  write(stream: TerminalStream, data: Buffer): void {
    if (this.ended) return
    this.append(this.decoders[stream].write(data))
  }

  end(): void {
    if (this.ended) return
    this.ended = true
    this.append(this.decoders.stdout.end(), false)
    this.append(this.decoders.stderr.end(), false)
    this.flush()
  }

  private append(data: string, schedule = true): void {
    if (!data) return
    this.chunks.push(data)
    this.bufferedLength += data.length
    if (this.bufferedLength >= this.maxBatchLength) {
      this.flush()
      return
    }
    if (schedule && !this.timer) this.timer = setTimeout(() => this.flush(), this.flushDelay)
  }

  private flush(): void {
    if (this.timer) clearTimeout(this.timer)
    this.timer = undefined
    if (this.bufferedLength === 0) return
    const output = this.chunks.join('')
    this.chunks = []
    this.bufferedLength = 0
    this.emit(output)
  }
}
