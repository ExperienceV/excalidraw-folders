/// <reference types="vite/client" />
import type { BoardsApi } from "../../shared/types"

declare global {
  interface Window {
    boards: BoardsApi
    EXCALIDRAW_ASSET_PATH?: string | string[]
  }
}
