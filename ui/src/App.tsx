import { Layout } from './components/Layout';
import { DashboardPage } from './pages/DashboardPage';
import { RunsPage } from './pages/RunsPage';
import { SessionsPage } from './pages/SessionsPage';
import { SettingsPage } from './pages/SettingsPage';

export default function App() {
  return (
    <Layout>
      {(page) => {
        switch (page) {
          case 'dashboard': return <DashboardPage />;
          case 'runs':      return <RunsPage />;
          case 'sessions':  return <SessionsPage />;
          case 'settings':  return <SettingsPage />;
          default:          return null; // Layout renders the built-in placeholder
        }
      }}
    </Layout>
  );
}
