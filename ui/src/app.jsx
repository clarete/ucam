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

import { useAuthState } from './hooks/useAuthState';

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
      <ListItemText
        primary={primary}
        style={{ overflow: 'hidden', textOverflow: 'ellipsis', marginRight: 20 }}
      />
      <ListItemSecondaryAction>
        {isConnectedTo &&
         <IconShell>
           <Avatar>
             <StopIcon />
           </Avatar>
         </IconShell>}

        {!isConnectedTo && caps.map(c =>
          ['produce:audio', 'produce:video'].includes(c) &&
            <IconShell key={`key-cap-${primary}-${c}`}>
              <Avatar>
                {c === 'produce:video' && <VideocamIcon style={iconStyle} />}
                {c === 'produce:audio' && <MicIcon style={iconStyle} />}
              </Avatar>
            </IconShell>
        )}
      </ListItemSecondaryAction>
    </ListItem>
  );
}

const ClientCardShell = styled.div`
  display: grid;
  margin: 0;
  padding: 0px 10px 10px 10px;
  place-items: center center;

  & .canvasEl {
    background-color: #aaa;
    border-radius: 5px;
  }
`;

function ClientCard({ jid }) {
  const { api } = React.useContext(store);
  const [loading, setLoading] = React.useState(false);
  const [videoEl, setVideoEl] = React.useState(null);
  // const callStatus = api.useCallStatus(videoEl, setLoading);

  // The render method won't show the video tag unless loading is
  // false. With that, this method ends up also depending on the loading
  // flag to be false as well.
  const canvasRefCallback = React.useCallback(node => {
    if (node !== null) {
      setVideoEl(node);
      setLoading(false);
      // api.saveVideoElementForJID(jid, node);
      // api.createPeerConnection(jid);
      // api.sendOffer(jid);
    }
  }, []);

  // Entry point of the WebRTC conversation. We create a peer connection
  // for intermediating the conversation with the peer identified by the
  // `client' parameter received above.
  // React.useEffect(() => {
  //   // api.(event) => {
  //   //   console.log('Add stream');
  //   //   videoEl.autoplay = true;
  //   //   videoEl.srcObject = event.stream;
  //   // }
  //   // api.createPeerConnection(jid);
  //   // api.sendOffer(jid);
  //   // api.addPendingCandidates(jid);
  // }, []);

  return (
    <Paper>
      <ClientCardShell>
        <h2>{jid}</h2>

        {loading && <SpinnerIcon />}

        {!loading &&
         <video
           autoPlay
           playsInline
           className="canvasEl"
           ref={canvasRefCallback}>
         </video>}
      </ClientCardShell>
    </Paper>
  );
}

function ConnectedClients({ connectedTo }) {
  return (
    <Grid container justify="center" spacing={2}>
      {Object.keys(connectedTo).map((jid) =>
        <Grid item key={`connected-client-${jid}`}>
          <ClientCard jid={jid} />
        </Grid>)}
    </Grid>
  );
}

function NoClientConnectedMessage() {
  return (
    <Typography component="h1" variant="h5" color="textSecondary" align="center">
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

  // // Feed the initial list of available clients
  // React.useEffect(() => {
  //   api.listClients().then(clients => {
  //     setClientList(clients);
  //     setLoading(false);
  //   });
  // }, []);

  // // Update the list of available clients upon websocket event
  // React.useEffect(() => {
  //   setClientList(api.state.clientList);
  // }, [api.state.clientList]);

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
              {Object.keys(api.state.connectedTo).length === 0 && <NoClientConnectedMessage />}
              {Object.keys(api.state.connectedTo).length > 0 && <ConnectedClients connectedTo={api.state.connectedTo} />}
            </ListClientScreenShell>
          </Grid>
        </Grid>
      </Container>
    </CenterCenterShell>
  );
}

function AuthForm() {
  const { authState, auth } = useAuthState();
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

          {authState === AuthState.Unauthorized &&
           <Error>
             Unauthorized.
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
  const { authState } = useAuthState();
  switch (authState) {
  case AuthState.Loading:
    return <Loading />;
  case AuthState.Authenticated:
    return <ListClientsScreen />;
  default:
    return <AuthForm />;
  }
}
