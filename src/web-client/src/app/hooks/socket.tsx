import { useState } from "react";

export enum ConnectionStatus {
  Off = "Off",
  Connecting = "Connecting",
  Connected = "Connected",
  Failed = "Failed",
}

export type SocketRequest<T> = {
  userId: string | null;
  body: T;
};
export type SocketResponse<T> = {
  userId: string | null;
  body: T;
};

// TODO: Can I not just make a class...???
export type SocketHook<RQ, RS> = {
  // Fields
  connectionStatus: ConnectionStatus;
  // Methods
  connect: (
    url: string,
    onOpenRequestProvider: () => RQ,
    onMessage: (message: RS) => void,
  ) => void;
  send: (msg: RQ) => void;
  close: () => void;
};

export default function useWebSocket<RQ, RS>(): SocketHook<RQ, RS> {
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>(
    ConnectionStatus.Off,
  );
  const [socket, setSocket] = useState<WebSocket | null>(null);
  const [userId, setUserId] = useState<string | null>(null);

  const send = (socket: WebSocket, request: RQ) => {
    const payload: SocketRequest<RQ> = {
      userId,
      body: request,
    };
    const as_string = JSON.stringify(payload);
    socket.send(as_string);
    console.log("Sent " + as_string);
  };

  const connectWebSocket = (
    url: string,
    onOpenRequestProvider: () => RQ | null,
    onMessage: (response: RS) => void,
  ) => {
    const newSocket = new WebSocket(url);
    setConnectionStatus(ConnectionStatus.Connecting);

    // TODO: Why does this fire twice?
    newSocket.onopen = () => {
      setConnectionStatus(ConnectionStatus.Connected);
      const request = onOpenRequestProvider();
      console.log("ABC", url, newSocket);
      if (request != null) {
        send(newSocket, request);
      }
    };

    newSocket.onerror = (event: Event) => {
      console.log("Error: ", event);
      setConnectionStatus(ConnectionStatus.Failed);
    };

    newSocket.onmessage = (event) => {
      const message = JSON.parse(event.data) as SocketResponse<RS>;
      if (!userId) {
        setUserId(message.userId);
      }
      onMessage(message.body);
    };

    // Fires when socket is closed by SERVER
    newSocket.onclose = (event: CloseEvent) => {
      // Normal closure
      if (event.code === 1000 && event.wasClean) {
        setConnectionStatus(ConnectionStatus.Off);
        return;
      }

      // Otherwise, try to reconnect
      // TODO: this should be specified by the hook user
      setTimeout(() => {
        connectWebSocket(url, onOpenRequestProvider, onMessage);
      }, 5000);
    };

    setSocket(newSocket);
  };

  return {
    connectionStatus: connectionStatus,
    connect: connectWebSocket,
    send: (msg: RQ) => {
      if (socket != null) {
        send(socket, msg);
      } else {
        console.log(
          "Tried to send " + JSON.stringify(msg) + " but socket is null",
        );
      }
    },
    close: () => {
      if (socket) {
        socket.close(1000, "User closed socket");
      }
    },
  };
}
