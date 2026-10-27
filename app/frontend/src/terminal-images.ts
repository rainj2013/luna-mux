import { ImageAddon } from '@xterm/addon-image'
import type { IDisposable, Terminal } from '@xterm/xterm'

class BoundedImageAddon extends ImageAddon {
  private budgetCheck?: IDisposable

  override activate(term: Terminal): void {
    super.activate(term)
    // The addon evicts on insertion, but font changes can create additional
    // scaled canvases during rendering. Reapply its public FIFO budget after
    // that work too; the following refresh removes evicted image tiles.
    this.budgetCheck = term.onRender(() => {
      if (this.storageUsage <= this.storageLimit) return
      this.storageLimit = this.storageLimit
      term.refresh(0, term.rows - 1)
    })
  }

  override dispose(): void {
    this.budgetCheck?.dispose()
    super.dispose()
  }
}

/** Images travel in the existing terminal stream, including over SSH/WSL.
 * Advertise SIXEL and cell metrics so TUIs can discover support themselves;
 * do not spoof Kitty support or force an Agent's protocol preference.
 * Each terminal owns its addon and releases the image layer on disposal. */
export function configureTerminalImages(term: Terminal): ImageAddon {
  const images = new BoundedImageAddon({
    sixelSupport: true,
    enableSizeReports: true,
    // Bound decoded pixels, encoded input and retained canvases per pane.
    pixelLimit: 2048 * 2048,
    sixelSizeLimit: 16 * 1024 * 1024,
    storageLimit: 32,
    // Only enable the protocol required by our TUIs.
    iipSupport: false
  })
  term.loadAddon(images)
  return images
}
