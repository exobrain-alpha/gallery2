import { useMemo } from "react";
import { CarouselView } from "./features/gallery/CarouselView";
import { GalleryView } from "./features/gallery/GalleryView";
import { SettingsView } from "./features/settings/SettingsView";
import "./App.css";

function App() {
  const view = useMemo(() => {
    const params = new URLSearchParams(window.location.search);
    return params.get("view") || "gallery";
  }, []);

  if (view === "settings") return <SettingsView />;
  if (view === "carousel") return <CarouselView />;
  return <GalleryView />;
}

export default App;
