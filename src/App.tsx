import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo } from "react";
import { CarouselView } from "./features/gallery/CarouselView";
import { GalleryView } from "./features/gallery/GalleryView";
import { SettingsView } from "./features/settings/SettingsView";
import "./App.css";

function App() {
  const view = useMemo(() => {
    const params = new URLSearchParams(window.location.search);
    return params.get("view") || "gallery";
  }, []);

  useEffect(() => {
    if (view === "desktop") return;

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      invoke("set_current_window_fullscreen", { fullscreen: false }).catch((error) => {
        console.error("Failed to exit fullscreen", error);
      });
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [view]);

  if (view === "settings") return <SettingsView />;
  if (view === "desktop") return <CarouselView desktopBackground />;
  if (view === "carousel") return <CarouselView />;
  return <GalleryView />;
}

export default App;
