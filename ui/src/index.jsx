import * as React from "react";
import * as ReactDOM from "react-dom";
import CssBaseline from '@material-ui/core/CssBaseline';

import App from './app';
import { ContextProvider } from './context';

ReactDOM.render(
  <ContextProvider>
    <CssBaseline />
    <App />
  </ContextProvider>,
  document.getElementById("mounting-point")
);
