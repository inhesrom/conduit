import { ConduitClient } from "@conduit/shared";
import { applyEvent } from "./state/apply-event";
import { setStore } from "./state/store";

// Same-origin: connect to /ws on whatever host served the page. Dev proxies
// it to the daemon; production serves it directly. wss when the page is https.
const proto = location.protocol === "https:" ? "wss" : "ws";
const url = import.meta.env.VITE_CONDUIT_WS_URL ?? `${proto}://${location.host}/ws`;

export const client = new ConduitClient({ url });

client.onEvent(applyEvent);
client.onStatus((conn) => setStore("conn", conn));
client.connect();
