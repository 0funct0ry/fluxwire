---
title: Quick start
description: Open a Workspace, send a request, and read the response and timing waterfall.
---

This walkthrough uses the sample collection in the repository, `fixtures/payments-api`. Any folder works.

1. **Open a Workspace.** Launch Fluxwire and choose **Open folder**, or run `fluxwire path/to/folder` from a terminal.
   A *Workspace* is the folder you open; it may hold one or more Collections.
2. **Pick a Request.** Select *Get payment* in the sidebar. A single click opens a preview tab; double-click pins it.
3. **Choose an Environment.** Press <kbd>Cmd/Ctrl+E</kbd> and select `local`. The `{{baseUrl}}` variable in the URL
   bar now resolves.
4. **Send.** Press <kbd>Cmd/Ctrl+Enter</kbd>. Sending never requires saving: the app uses the text in the editor.
5. **Read the response.** The Body tab pretty-prints JSON; Headers, Cookies, Timing, and Tests have their own tabs.
   The waterfall shows DNS, Connect, TLS, Waiting, and Download phases.
6. **Look at the file.** Use **Open as text** to see the `.flux.md` source. Edits you make in the form touch only the
   lines they own, so `git diff` stays small.

## Create a request

Press <kbd>Cmd/Ctrl+N</kbd> to open the New request dialog. Choose a method, enter a URL, and pick a folder. You can
also paste a cURL command; Fluxwire parses it into a request.

## Run it from a terminal

```sh
fluxwire send fixtures/payments-api/payments/get-payment.flux.md --env local
```

## Next steps

- [Your first collection](../first-collection/)
- [Environments and secrets](../../guides/environments-and-secrets/)
- [Scripting](../../guides/scripting/)
