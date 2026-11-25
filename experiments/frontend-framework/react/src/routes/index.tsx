import { createFileRoute, Link } from '@tanstack/react-router';

export const Route = createFileRoute('/')({ component: Home });

function Home() {
  return (
    <main className="settings">
      <h1>SourceOrigin, in React with TanStack Start</h1>
      <p className="lead">The two screens of the framework experiment.</p>
      <ul>
        <li>
          <Link to="/changes/$change" params={{ change: '1' }}>
            Review change 1
          </Link>
        </li>
        <li>
          <Link to="/settings">Account settings</Link>
        </li>
      </ul>
    </main>
  );
}
