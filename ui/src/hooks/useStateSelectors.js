import { useContext } from 'react';
import { appContext } from '../context';

/// Pulls most useful fields of the application context.  Notice: this
/// is mostly intended to be used by *other hooks* instead of letting
/// the application code itself use it.
export function useStateSelectors() {
  const { state } = useContext(appContext);

  /// Returns true if a client isn't disconnected
  const wsIsConnected = jid => {
    return state.wsStateByID[jid] !== undefined;
  };

  /// Returns all peers that aren't disconnected
  const wsConnectedPeers = () => {
    return Object.keys(state.wsStateByID).filter(jid => wsIsConnected(jid));
  };

  return {
    wsIsConnected,
    wsConnectedPeers,
  };
}
