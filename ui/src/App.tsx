import { Layout } from './components/Layout';
import { DashboardPage } from './pages/DashboardPage';
import { RunsPage } from './pages/RunsPage';
import { SessionsPage } from './pages/SessionsPage';

export default function App() {
  return (
    <Layout>
      {(page) => {
        switch (page) {
          case 'dashboard': return <DashboardPage />;
          case 'runs':      return <RunsPage />;
          case 'sessions':  return <SessionsPage />;
          default:          return null; // Layout renders the placeholder
        }
      }}
    </Layout>
  );
}
