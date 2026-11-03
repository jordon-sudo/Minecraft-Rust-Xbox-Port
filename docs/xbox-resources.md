# Getting prepared resources onto an Xbox

Horizon never ships Minecraft's files. On every platform it builds its own
prepared resources from the player's own copy of the game, and Xbox is no
different — except that a UWP app cannot reach outside its sandbox, so the
files have to be put somewhere it may read.

That place is the app's **`LocalState\resources`** folder.

## Why it is not in the package

Two reasons, and the first is the one that matters:

1. Putting Mojang's files in a package Horizon distributes would be
   redistributing them. The repository's rules forbid committing them, and the
   same reasoning forbids shipping them.
2. The package install folder is read-only on the console, so resources could
   not be updated or re-prepared without reinstalling the whole app.

So the package carries the binary, and the resources arrive separately.

## Step 1: prepare them on your PC

With your own copy of Minecraft 26.3:

```sh
cargo run --release -p horizon-prepare -- --prepare --server-jar ~/Downloads/server.jar
```

This writes `.local/horizon-client/26.3`. It is the same folder the desktop
build runs against with `--resources`, so if the game already works on your
PC, you have it.

## Step 2: push them to the console

```sh
python3 tools/xbox_push_resources.py --host <console-ip> --user <user> --password <password>
```

The script walks `.local/horizon-client/26.3` and uploads each file through
Device Portal's filesystem API into `LocalState\resources`, keeping the
directory structure. `--dry-run` lists what it would send without sending it.

It talks to the same REST API Device Portal's own file explorer uses, so
nothing extra is needed on the console beyond remote access being on.

### If the package name changed

The script defaults to the package family name that follows from the
manifest's `Identity Name` and the publisher hash:

```
dev.horizonlab.Horizon_8wekyb3d8bbwe
```

If you changed `Identity Name` or signed with a different publisher, that
suffix changes. Device Portal's app list shows the real package full name;
pass it with `--package`.

## Doing it by hand instead

Device Portal can do this without the script, which is worth knowing when
something fails and you want to see the folder:

1. Open `https://<console-ip>:11443`.
2. Go to **Home**, find Horizon in the app list, and open its **file
   explorer** (sometimes under the app's **…** menu, sometimes a folder icon).
3. Navigate into **LocalState**.
4. Create a folder called `resources`.
5. Upload the contents of `.local/horizon-client/26.3` into it.

The structure must be preserved: `LocalState\resources\assets\...`, not
everything flattened into one folder.

## Checking it worked

Launch Horizon. If it still shows the "prepared resources not found" screen,
the files are in the wrong place. The two usual mistakes:

- They went into `LocalState` directly instead of `LocalState\resources`.
- The directory structure was flattened by a manual upload.

## Security note

Device Portal uses HTTPS with a **self-signed, per-console** certificate, so
there is nothing to chain it to and the script does not verify it by default.
The connection is still encrypted; it is the console's identity that is
unverified. Use it on a network you trust, and do not expose port 11443 to the
internet. If you want verification, export the console's certificate from
Device Portal and pass it with `--ca-bundle`.
