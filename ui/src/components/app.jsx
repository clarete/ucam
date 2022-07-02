import * as React from 'react';
import { useEffect, useState, useRef } from 'react';

import styled from 'styled-components';
import Avatar from '@material-ui/core/Avatar';
import Grid from '@material-ui/core/Grid';
import FormControlLabel from '@material-ui/core/FormControlLabel';
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

import CenterCenterShell from './centercentershell'
import Loading from './loading';
import SpinnerIcon from './spinner';
import AuthForm from './authform';

import { useAppContext } from '../hooks/useAppContext';
import { useAuthState } from '../hooks/useAuthState';
import { useWebSocket } from '../hooks/useWebSocket';

import { actions } from '../context/reducers';
import { AuthState } from '../services/auth';
import * as serverAPI from '../services/api';

import adapter from 'webrtc-adapter';

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

function ClientItem({ jid, caps }) {
  const {
    state,
    isClientConnected,
    connectToClient,
    disconnectFromClient,
  } = useAppContext();
  const isConnectedTo = isClientConnected(jid);
  const handleItemClick = () => isConnectedTo
    ? disconnectFromClient(jid)
    : connectToClient(jid);

  return (
    <ListItem button component="li" onClick={handleItemClick}>
      <ListItemText
        primary={jid}
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
            <IconShell key={`key-cap-${jid}-${c}`}>
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
    border-color: #000;
  }
`;

function ClientCard({ jid }) {
  const { dispatch, state, webSocketSend } = useAppContext();
  const [loading, setLoading] = useState(false);
  const videoEl = useRef(null);

  // Entry point of the WebRTC conversation. We create a peer connection
  // for intermediating the conversation with the peer identified by the
  // `client' parameter received above.
  useEffect(() => {
    const pc = new RTCPeerConnection({
      // iceServers: [{
      //   urls: "stun:stun.l.google.com"
      // }]
    });

    pc.onconnectionstatechange = event => {
      console.log(`onconnectionstatechange: state=${pc.connectionState}`);
      console.dir(event);
    };

    pc.oniceconnectionstatechange = event => {
      console.log(`oniceconnectionstatechange: state=${pc.iceConnectionState}`);
      console.dir(event);

      if (pc.iceConnectionState === "failed") {
        console.log(`restart ice`);
        pc.restartIce();
      }
    };

    pc.onicecandidate = event => {
      console.log(`onicecandidate`);
      console.dir(event);

      if (event.candidate !== null) {
        webSocketSend(jid, { ice: event.candidate });
      }
    };

    pc.ontrack = event => {
      console.log(`ontrack: ${videoEl.current}`);
      if (videoEl.current && videoEl.current.srcObject !== event.streams[0]) {
        console.dir(event);
        videoEl.current.srcObject = event.streams[0];
      }
    };

    const createOffer = () => {
      pc.createOffer({
        offerToReceiveAudio: false, //true,
        offerToReceiveVideo: true,
        iceRestart: false, //true,
      })
        .then(sdp => pc.setLocalDescription(sdp))
        .then(() => webSocketSend(jid, { sdp: pc.localDescription }))
        .catch(error => console.error('Send offer failed: ', error));
    };

    pc.onnegotiationneeded = event => {
      console.log(`onnegotiationneeded`);
      console.dir(event);
      createOffer();
    };

    webSocketSend(jid, 'peerrequestcall');

    dispatch({ type: actions.WRTC_PEER_CONNECTION, jid, pc })

  }, []);

  return (
    <Paper>
      <ClientCardShell>
        <h2>{jid}</h2>

        {loading && <SpinnerIcon />}

        {!loading &&
         <video
           controls
           autoPlay
           playsInline
           className="video"
           ref={videoEl}>
         </video>
        }
      </ClientCardShell>
    </Paper>
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
  const [loading, setLoading] = useState(true);
  const { state, dispatch, getConnectedClients } = useAppContext();
  const connectedClients = getConnectedClients();

  useWebSocket();
  useEffect(() => {
    serverAPI
      .getRoster(state.authToken)
      .then(roster => {
        dispatch({ type: actions.ROSTER_LIST, roster });
        setLoading(false);
      });
  }, []);

  if (loading)
    return <Loading />;

  if (Object.entries(state.roster).length === 0)
    return <NobodyToTalkMessage />;

  return (
    <CenterCenterShell>
      <Container component="main" maxWidth="lg">
        <Grid container spacing={8}>
          <Grid item xs={12}>
            <h1>What do you want to see</h1>
          </Grid>

          <Grid item xs={4}>
            <List>
              {Object.entries(state.roster).map(([jid, peer], i) =>
                <div key={`key-client-${jid}`}>
                  <ClientItem jid={jid} caps={peer.capabilities.sort()} />
                  <Divider component="li" />
                </div>)}
            </List>
          </Grid>

          <Grid item xs={8}>
            <ListClientScreenShell>
              {connectedClients.length === 0 &&
               <NoClientConnectedMessage />}

              {connectedClients.length > 0 &&
               <Grid container justify="center" spacing={2}>
                 {connectedClients.map(jid =>
                   <Grid item key={`connected-client-${jid}`}>
                     <ClientCard jid={jid} />
                   </Grid>)}
               </Grid>}

            </ListClientScreenShell>
          </Grid>
        </Grid>
      </Container>
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
