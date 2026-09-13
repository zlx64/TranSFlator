import { Routes, Route, Navigate } from "react-router-dom";
import Layout from "./components/Layout";
import Health from "./pages/Health";
import Library from "./pages/Library";
import Media from "./pages/Media";
import Jobs from "./pages/Jobs";
import Settings from "./pages/Settings";
import AuthGate from "./components/AuthGate";

export default function App() {
  return (
    <AuthGate>
      <Layout>
        <Routes>
          <Route path="/" element={<Navigate to="/library" replace />} />
          <Route path="/library" element={<Library />} />
          <Route path="/media" element={<Media />} />
          <Route path="/jobs" element={<Jobs />} />
          <Route path="/settings" element={<Settings />} />
          <Route path="/health" element={<Health />} />
          <Route path="*" element={<Navigate to="/library" replace />} />
        </Routes>
      </Layout>
    </AuthGate>
  );
}
