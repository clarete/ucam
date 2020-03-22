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
import Divider from '@material-ui/core/Divider';
import List from '@material-ui/core/List';
import ListItem from '@material-ui/core/ListItem';
import ListItemText from '@material-ui/core/ListItemText';
import ListSubheader from '@material-ui/core/ListSubheader';
import ListItemAvatar from '@material-ui/core/ListItemAvatar';
import ListItemSecondaryAction from '@material-ui/core/ListItemSecondaryAction';
import VideocamIcon from '@material-ui/icons/Videocam';
import MicIcon from '@material-ui/icons/Mic';

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

const IconShell = styled.div`
  width: 24px;
  height: 24px;
  float: left;
  margin-left: 4px;

  .MuiAvatar-root {
    width: 25px;
    height: 25px;
  }

  .MuiAvatar-colorDefault {
    background-color: #ccc;
  }
`;

const StreamingClientShell = styled.div`
  & video {
    border: solid 1px black;
  }
`;

function StreamingClient({ clientId }) {
  return (
    <StreamingClientShell>
      <h2>{clientId}</h2>
      <video id={`stream-${clientId}`} autoPlay={true} playsInline={true}>
        Your browser doesn't support video
      </video>
      <div>Status: <b id={`status-${clientId}`}>unknown</b></div>
      <div>Peer ID: <b id={`peer-id-${clientId}`}>unknown</b></div>
      <div><textarea id={`text-id-${clientId}`} cols="40" rows="4"></textarea></div>
    </StreamingClientShell>
  );
}

function StreamingScreen() {
  const { api } = React.useContext(store);
  return (
    <CenterCenterShell>
      {api.state.connectedTo.map(cid =>
        <StreamingClient key={`key-cli-${cid}`} clientId={cid} />)}
    </CenterCenterShell>
  );
}

const iconStyle = { width: 16, height: 16 };

function ClientItem({ primary, caps }) {
  const { api } = React.useContext(store);
  return (
    <ListItem button component="li" onClick={() => api.connectTo(primary)}>
      <ListItemText primary={primary} />
      <ListItemSecondaryAction>
        {caps.map(c =>
          ['s:audio', 's:video'].includes(c) &&
            <IconShell key={`key-cap-${primary}-${c}`}>
              <Avatar>
                {c === 's:video' && <VideocamIcon style={iconStyle} />}
                {c === 's:audio' && <MicIcon style={iconStyle} />}
              </Avatar>
            </IconShell>
        )}
      </ListItemSecondaryAction>
    </ListItem>
  );
}

function ListClientsScreen() {
  const { api } = React.useContext(store);
  const [clientList, setClientList] = React.useState({});
  React.useEffect(() => {
    api.listClients().then(clients => setClientList(clients));
  }, []);
  return (
    <CenterCenterShell>
      <Container component="main" maxWidth="xs">
        <h1>What do you want to see</h1>
        <List>
          {Object.entries(clientList).map(([jid, caps], i) =>
            <div key={`key-client-${jid}`}>
              <ClientItem primary={jid} caps={caps} />
              <Divider component="li" />
            </div>)}
        </List>
      </Container>
    </CenterCenterShell>
  );
}

function AuthForm() {
  const { api } = React.useContext(store);
  const { register, handleSubmit, errors } = useForm();
  const onSubmit = async data => api.startSession(data);

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

          {api.authState() === AuthState.Unauthorized &&
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

function MainScreen() {
  const { api } = React.useContext(store);
  return api.isConnectedToSomeone()
    ? <StreamingScreen />
    : <ListClientsScreen />;
}

function Loading() {
  return (
    <CenterCenterShell>
      <SpinnerIcon />
    </CenterCenterShell>
  );
}

export default function App() {
  const { api } = React.useContext(store);
  switch (api.authState()) {
  case AuthState.Loading:
    return <Loading />;
  case AuthState.Authenticated:
    return <MainScreen />;
  default:
    return <AuthForm />;
  }
}
