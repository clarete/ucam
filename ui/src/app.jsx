import * as React from 'react';
import styled from 'styled-components';

import Avatar from '@material-ui/core/Avatar';
import Grid from '@material-ui/core/Grid';
import Button from '@material-ui/core/Button';
import TextField from '@material-ui/core/TextField';
import FormControlLabel from '@material-ui/core/FormControlLabel';
import LockOutlinedIcon from '@material-ui/icons/LockOutlined';
import Typography from '@material-ui/core/Typography';
import Container from '@material-ui/core/Container';
import Divider from '@material-ui/core/Divider';
import Paper from '@material-ui/core/Paper';

import List from '@material-ui/core/List';
import ListItem from '@material-ui/core/ListItem';
import ListItemText from '@material-ui/core/ListItemText';
import ListSubheader from '@material-ui/core/ListSubheader';
import ListItemAvatar from '@material-ui/core/ListItemAvatar';
import ListItemSecondaryAction from '@material-ui/core/ListItemSecondaryAction';
import VideocamIcon from '@material-ui/icons/Videocam';
import MicIcon from '@material-ui/icons/Mic';
import StopIcon from '@material-ui/icons/Stop';

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

const iconStyle = { width: 16, height: 16 };

function ClientItem({ primary, caps }) {
  const { api } = React.useContext(store);
  const isConnectedTo = api.isConnectedTo(primary);
  const handleItemClick = () => isConnectedTo
    ? api.disconnectFrom(primary)
    : api.connectTo(primary);
  return (
    <ListItem button component="li" onClick={handleItemClick}>
      <ListItemText primary={primary} />
      <ListItemSecondaryAction>

        {isConnectedTo &&
         <IconShell>
           <Avatar>
             <StopIcon />
           </Avatar>
         </IconShell>}

        {!isConnectedTo && caps.map(c =>
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

const ClientCardShell = styled.div`
  padding: 0px 10px 10px 10px;

  & .canvasEl {
    background-color: #aaa;
    border-radius: 5px;
  }
`;

function ClientCard({ client }) {
  const canvasRef = React.useRef();
  return (
    <Paper>
      <ClientCardShell>
        <h2>{client}</h2>
        <video className="canvasEl" ref={canvasRef} autoPlay={true} playsInline={true}></video>
      </ClientCardShell>
    </Paper>
  );
}

function ConnectedClients({ list }) {
  return (
    <Grid container justify="center" spacing={2}>
      {list.map((c) =>
        <Grid item key={`connected-client-${c}`}>
          <ClientCard client={c} />
        </Grid>)}
    </Grid>
  );
}

function NoClientConnectedMessage() {
  return (
    <Typography component="h1" variant="h5" color="textSecondary">
      Click in one of the clients listed on the left
    </Typography>
  );
}

function NobodyToTalkMessage() {
  return (
    <CenterCenterShell>
      <Typography component="h1" variant="h5" color="textSecondary">
        No client is connected to the server
      </Typography>
    </CenterCenterShell>
  );
}

const ListClientScreenShell = styled.div`
  display: grid;
  background-color: #eee;
  border-radius: 10px;
  min-height: 25vh;
  padding: 10px;
  margin: 0;
  place-items: center center;
`;

function ListClientsScreen() {
  const { api } = React.useContext(store);
  const [loading, setLoading] = React.useState(true);
  const [clientList, setClientList] = React.useState({});
  const [connectedClients, setConnectedClients] = React.useState([]);
  // Feed the initial list of available clients
  React.useEffect(() => {
    api.listClients().then(clients => {
      setClientList(clients);
      setLoading(false);
    });
  }, []);

  // Update the list of available clients upon websocket event
  React.useEffect(() => {
    setClientList(api.state.clientList);
  }, [api.state.clientList]);

  // Feed the list of already connected (or connecting) clients
  React.useEffect(() => {
    setConnectedClients(api.connectedClients());
  }, [api.state.connectedTo]);

  if (loading)
    return <Loading />;

  if (Object.entries(clientList).length === 0)
    return <NobodyToTalkMessage />

  return (
    <CenterCenterShell>
      <Container component="main" maxWidth="lg">
        <Grid container spacing={8}>
          <Grid item xs={12}>
            <h1>What do you want to see</h1>
          </Grid>

          <Grid item xs={4}>
            <List>
              {Object.entries(clientList).map(([jid, caps], i) =>
                <div key={`key-client-${jid}`}>
                  <ClientItem primary={jid} caps={caps.sort()} />
                  <Divider component="li" />
                </div>)}
            </List>
          </Grid>

          <Grid item xs={8}>
            <ListClientScreenShell>
              {connectedClients.length === 0 && <NoClientConnectedMessage />}
              {connectedClients.length > 0 && <ConnectedClients list={connectedClients} />}
            </ListClientScreenShell>
          </Grid>
        </Grid>
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
    return <ListClientsScreen />;
  default:
    return <AuthForm />;
  }
}
