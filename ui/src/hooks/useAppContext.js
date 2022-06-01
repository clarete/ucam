import { useContext } from 'react';
import { appContext } from '../context';

/// Pulls most useful fields of the application context.  Notice: this
/// is mostly intended to be used by *other hooks* instead of letting
/// the application code itself use it.
export function useAppContext() {
  const { state, dispatch } = useContext(appContext);
  return { state, dispatch };
}
