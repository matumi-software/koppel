import { useAuth } from '@/auth/auth-context';

export function SignOutButton() {
  const { signOut } = useAuth();
  return <button onClick={() => void signOut()}>Sign out</button>;
}

export function RecheckButton() {
  const { recheck } = useAuth();
  return <button onClick={recheck}>Recheck</button>;
}
