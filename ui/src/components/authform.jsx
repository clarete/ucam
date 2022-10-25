import * as React from 'react';
import styled from 'styled-components';
import { useForm } from 'react-hook-form';

import Avatar from '@material-ui/core/Avatar';
import Button from '@material-ui/core/Button';
import Container from '@material-ui/core/Container';
import TextField from '@material-ui/core/TextField';
import Typography from '@material-ui/core/Typography';
import LockOutlinedIcon from '@material-ui/icons/LockOutlined';

import CenterCenterShell from './centercentershell'
import { useAuthState } from '../hooks/useAuthState';
import { AuthState } from '../services/auth';

const AvatarShell = styled.div`
  & .avatar {
    float: right;
  }
`;

const Error = styled.div`
  padding: 0 0 10px 0;
  color: #ee2200;
`;

const ButtonShell = styled.div`
  margin-top: 15px;
`;

export default function AuthForm() {
  const { authState, authError, auth } = useAuthState();
  const { register, handleSubmit, errors } = useForm();
  const onSubmit = async formData => auth(formData);

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
        <form noValidate onSubmit={handleSubmit(onSubmit)}>
          <TextField
            variant="outlined"
            margin="normal"
            required
            fullWidth
            id="jid"
            label="User ID"
            name="jid"
            autoComplete="jid"
            autoFocus
            inputRef={register({ required: true })}
          />
          {errors.jid &&
           <Error>
             This field is required.
           </Error>}

          <TextField
            variant="outlined"
            margin="normal"
            required
            fullWidth
            type="password"
            id="password"
            label="Password"
            name="password"
            inputRef={register({ required: true })}
          />
          {errors.password &&
           <Error>
             This field is required.
           </Error>}

          {authState === AuthState.Failed &&
           <Error>
             {authError.httpString}.
           </Error>}

          <ButtonShell>
            <Button
              type="submit"
              fullWidth
              variant="contained"
              color="primary"
            >
              Sign In
            </Button>
          </ButtonShell>
        </form>
      </Container>
    </CenterCenterShell>
  );
}
