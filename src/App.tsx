import { useMemo } from "react";
import { GalleryView } from "./features/gallery/GalleryView";
import { SettingsView } from "./features/settings/SettingsView";
import "./App.css";

function App() {
  const view = useMemo(() => {
    const params = new URLSearchParams(window.location.search);
    return params.get("view") || "gallery";
  }, []);

  return view === "settings" ? <SettingsView /> : <GalleryView />;
}

export default App;
