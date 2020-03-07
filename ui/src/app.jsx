import * as React from 'react';
import styled from 'styled-components';

import Avatar from '@material-ui/core/Avatar';
import Grid from '@material-ui/core/Grid';
import Button from '@material-ui/core/Button';
import TextField from '@material-ui/core/TextField';
import FormControlLabel from '@material-ui/core/FormControlLabel';
import Link from '@material-ui/core/Link';
import Box from '@material-ui/core/Box';
import LockOutlinedIcon from '@material-ui/icons/LockOutlined';
import Typography from '@material-ui/core/Typography';
import Container from '@material-ui/core/Container';

import { useForm } from 'react-hook-form';
import { store, AuthState } from './store';
import SpinnerIcon from './spinner';

const CenterCenterShell = styled.div`
  display: grid;
  height: 100vh;
  margin: 0;
  place-items: center center;
`;

const AvatarShell = styled.div`
  & .avatar {
    float: right;
  }
`;

const Error = styled.div`
  padding: 0 0 10px 0;
  color: #ee2200;
`;

function MainScreen() {
  return (
    <div>
      Should be something
    </div>
  );
}

function AuthForm() {
  const { api, state } = React.useContext(store);
  const { register, handleSubmit, errors } = useForm();

  return (
    <CenterCenterShell>
      <Container component="main" maxWidth="xs">
        <AvatarShell>
          <Avatar className="avatar">
            <LockOutlinedIcon />
          </Avatar>
          <Typography component="h1" variant="h5">
            Sign in
          </Typography>
        </AvatarShell>
        <form noValidate onSubmit={handleSubmit(api.auth)}>
          <TextField
            variant="outlined"
            margin="normal"
            required
            fullWidth
            id="email"
            label="Email Address"
            name="email"
            autoComplete="email"
            autoFocus
            inputRef={register({ required: true })}
          />
          {errors.email &&
           <Error>
             Email is required.
           </Error>}

          {state.auth.state === AuthState.Unauthorized &&
            <Error>
              Email unauthorized.
            </Error>}

          <Button
            type="submit"
            fullWidth
            variant="contained"
            color="primary"
          >
            Sign In
          </Button>
        </form>
      </Container>
    </CenterCenterShell>
  );
}

function Loading() {
  return (
    <CenterCenterShell>
      <SpinnerIcon />
    </CenterCenterShell>
  );
}

export default function App() {
  const { dispatch, state } = React.useContext(store);
  switch (state.auth.state) {
  case AuthState.Loading:
    return <Loading />;
  case AuthState.Authenticated:
    return <MainScreen />;
  default:
    return <AuthForm />;
  }
}
