import * as React from "react";
import * as ReactDOM from "react-dom";
import CssBaseline from '@material-ui/core/CssBaseline';

import { Provider } from './store';
import App from './app';

ReactDOM.render(
  <Provider>
    <CssBaseline />
    <App />
  </Provider>,
  document.getElementById("mounting-point")
);
