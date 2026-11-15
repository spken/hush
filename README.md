# Hush

A native Windows client for Spotify. Hush runs as a Spotify Connect device on your PC,
plays music through its own player, and shows your library, search and other devices.

This guide covers what you need to set up before Hush can use your Spotify account.
To work on the code, read [AGENTS.md](AGENTS.md).

---

## What you need

- **Windows 10 or 11**, 64-bit.
- **Spotify Premium.** Spotify streams audio to third-party players only for Premium
  accounts, and only Premium owners can create a working developer app.
- **A Spotify developer app** of your own, which gives you a *Client ID*. You make this
  once in Spotify's developer dashboard. It is free.
- **Port 8898 free on your PC.** Sign-in sends your browser back to Hush on
  `http://127.0.0.1:8898`. Nothing outside your PC can reach it.

---

## 1 · Install Hush

1. Download `hush.exe` from the
   [latest release](https://github.com/spken/hush/releases/latest).
2. Put it anywhere you like, for example `C:\Users\<you>\Apps\Hush\hush.exe`, and run it.
   If Windows SmartScreen warns you about an unknown publisher, choose **More info** →
   **Run anyway**.

Hush updates itself: **Settings → About** checks for a newer release, downloads it and
restarts.

---

## 2 · Create your Spotify developer app

Hush needs its own Client ID to read your library, search, and control your other
devices through Spotify's Web API. Spotify gives each developer a limited request quota,
so every Hush user brings their own app.

1. Go to the [Spotify Developer Dashboard](https://developer.spotify.com/dashboard) and log
   in **with the Premium account you will listen on**. Accept the developer terms if
   asked.
2. Click **Create app** and fill in the form:

   | Field | Value |
   | --- | --- |
   | App name | `Hush` (any name works; you see it once on the sign-in page) |
   | App description | Anything, for example `Personal desktop client` |
   | Website | Leave empty |
   | Redirect URIs | `http://127.0.0.1:8898/callback` — then click **Add** |
   | Which API/SDKs are you planning to use? | Tick **Web API** |

   Type the redirect URI exactly as shown. It must be `127.0.0.1`, not `localhost`, and
   must use `http`, port `8898` and the path `/callback`.
3. Accept the terms and click **Save**.
4. Open the app's **Settings** and copy the **Client ID**. You do **not** need the client
   secret; never paste it anywhere.

### Using a different Spotify account than the app owner

New developer apps run in *development mode*, which only lets accounts you list sign in.
The owner is allowed automatically. To let another account use your Client ID, open the
app in the dashboard → **User Management** → add that account's name and the email it
uses with Spotify. A development-mode app allows at most 5 users.

---

## 3 · First launch and sign-in

Hush signs in to Spotify twice, for two different jobs. Each sign-in opens your default
browser, asks you to approve, and ends on a page that says *"You can close this tab and
return to Hush."*

### a. Playback sign-in (first start)

On the first start, Hush opens your browser to sign in for playback. Approve it with your
Premium account. The approval page may name Spotify's own desktop app rather than yours;
that is expected, because Spotify only grants playback to its own client.

Hush then appears as **Hush** in the device list of every Spotify app on your account
(phone, web player, desktop). You can already play to it from those apps.

### b. Library sign-in (after adding your Client ID)

1. In Hush, open **Settings → Spotify → Client ID** and paste the Client ID from step 2.
2. Quit Hush and start it again. The Client ID takes effect on the next start.
3. Your browser opens again, this time showing **your** app's name and the permissions
   Hush asks for (below). Approve it.

Hush then loads your Liked Songs, saved albums and playlists, and turns on search and
controlling your other devices. The first sync of a large library takes a while; it runs
in the background. After that Hush syncs on its own, or when you click **Sync Now** on
the Library page.

After this, Hush starts without opening the browser. It signs in again only if Spotify
revokes its access.

### Permissions Hush asks for

| Permission | Lets Hush |
| --- | --- |
| `streaming` | Play music (playback sign-in) |
| `user-library-read`, `user-library-modify` | Read and change your Liked Songs and saved albums |
| `playlist-read-private`, `playlist-read-collaborative` | Read your playlists |
| `playlist-modify-private` | Change your private playlists |
| `user-read-playback-state`, `user-modify-playback-state` | See and control your other devices |
| `user-read-recently-played`, `user-top-read` | Read what you played recently and your top artists and tracks |

---

## What works without a Client ID

You can skip step 2 and still use Hush as a player: it signs in for playback, shows up as
a Spotify Connect device, and plays whatever you send to it from another Spotify app.
Your library, search and the device picker stay off until you add a Client ID.

---

## Settings

| Setting | What it does |
| --- | --- |
| **Client ID** | Your developer app's Client ID. Applies on the next start. |
| **Output** | The sound device Hush plays to. Applies at once. |
| **Normalize Volume** | Plays every track at a similar loudness. Applies on the next start. |
| **Gapless Playback** | Runs tracks into each other without a pause. Applies on the next start. |
| **Equalizer** | Ten bands with presets. Applies at once. |
| **About** | Shows the version and installs updates. |

---

## Your data and signing out

| What | Where |
| --- | --- |
| Library cache, settings, artwork cache and the playback sign-in | `%LOCALAPPDATA%\Hush` |
| The library sign-in (a refresh token) | Windows Credential Manager → **Windows Credentials** → **Generic Credentials**, the entry whose name contains `Hush` |

Hush never stores a password, and keeps the refresh token only in Credential Manager.

To sign out completely:

1. Quit Hush.
2. Delete the folder `%LOCALAPPDATA%\Hush`.
3. Remove the `Hush` entry from Credential Manager.
4. Optionally, revoke Hush's access at
   [spotify.com/account/apps](https://www.spotify.com/account/apps/).

The next start signs in from scratch.

---

## Troubleshooting

| What you see | Cause and fix |
| --- | --- |
| Spotify shows **INVALID_CLIENT: Invalid redirect URI** | The redirect URI in your dashboard app does not match. Set it to exactly `http://127.0.0.1:8898/callback` and save. |
| Spotify says the user is **not registered** for the app, or sign-in fails with 403 | The account you signed in with is not the app's owner and is not under **User Management**. Add it (step 2). |
| The browser never comes back to Hush | Another program is using port 8898, or a firewall blocks local connections. Close the other program and start Hush again. |
| Library and search stay empty | No Client ID is set, or Hush has not restarted since you set it. Check **Settings → Client ID** and restart. |
| *"Spotify's request limit was reached; showing saved data for now."* | Your developer account used up its Web API quota. Hush keeps working from saved data and recovers on its own, usually within an hour. The quota is shared by every app on your developer account, so do not run other developer apps on it. |
| *"Can't reach Spotify. Reconnecting; what's saved stays available."* | The playback connection dropped. Hush reconnects by itself; saved pages stay browsable meanwhile. |
| Nothing plays, or playback sign-in fails | The account is not Premium. |
| Starting Hush does nothing | Hush is already running. Only one copy runs at a time; look for its window or taskbar icon. |

### Limits from Spotify

Spotify restricts development-mode apps, and Hush is built around those limits:

- At most 5 Spotify accounts per developer app, and the owner must have Premium.
- Search returns 10 results per type at a time; **Show More** loads the next ten.
- The Web API gives no recommendations, related artists or radio, so Hush has none of
  these.

---

## Advanced: `.env` options

Hush reads a `.env` file from the folder it is started in, if one exists. See
[.env.example](.env.example).

| Variable | Effect |
| --- | --- |
| `HUSH_CATALOG=web-api` | Loads album, artist and playlist pages from the Web API instead of Spotify's own metadata service. Uses more of your quota; meant for troubleshooting. |
