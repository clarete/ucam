import { AuthState } from '../services/auth';

export const actions = {
  AUTH_LOADING: "AUTH_LOADING",
  AUTH_SUCCESS: "AUTH_SUCCESS",
  AUTH_FAILURE: "AUTH_FAILURE",
};

export function createReducer() {
  return (state, action) => {
    const { type, ...data } = action;

    console.log(`EVENT OF TYPE ${type}`);
    console.dir(data);

    switch (type) {
    case actions.AUTH_LOADING:
      return { ...state, authState: AuthState.Loading };
    case actions.AUTH_SUCCESS:
      return { ...state, ...data };
    case actions.AUTH_FAILURE:
      return { ...state, ...data  };
    default:
      throw new Error(`action type not known: ${type}`);
    }
  }
}
