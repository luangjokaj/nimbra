# Connecting your mail

Nimbra works with Gmail and Google Workspace, Outlook.com and Microsoft 365, and any IMAP account. Add one from the welcome screen, or later from **Settings › Accounts**.

- [Gmail](#gmail)
- [Other providers](#other-providers)
- [Several accounts](#several-accounts)

## Gmail

The builds on the Releases page sign in to Google and Microsoft as they are. The steps below are for building Nimbra yourself.

1. In Google Cloud Console, create a project, enable the Gmail API, and create an OAuth client of type "Desktop app". Keep the consent screen in Testing mode and add yourself as a test user.
2. Put the client in `~/.config/nimbra/oauth.env` (mode 600), never in the repo:

   ```sh
   NIMBRA_GOOGLE_CLIENT_ID=...
   NIMBRA_GOOGLE_CLIENT_SECRET=...
   ```

3. `cargo run`, then click **Connect Gmail**. Google opens in your browser.

Nimbra asks for the `gmail.modify` scope only. It covers reading, archiving, labelling and sending; it does not allow permanent deletion (the first permanent delete in Trash asks Google for that permission, and every one asks for confirmation). Nimbra sends only when you press Send. Signing in again takes only that same Google account: another one picked in the browser is refused before anything is saved.

Your Google profile photo comes from the People API (turn it on in your Cloud project), which tells your own photo from the letter Google draws.

## Other providers

**Add account** takes Google, or any address with a password (Zoho, iCloud, Fastmail, Yahoo, a company's own server). A Gmail or Google Workspace address can also be added this way with an [app password](https://myaccount.google.com/apppasswords), over IMAP and SMTP; Nimbra says so if Google wants one rather than your Google password. Nimbra finds the servers from the address (or you type them), checks that both take the password, and keeps the password in the keychain. A custom domain hosted at Google, Microsoft or Zoho is recognised from its mail (MX) records.

Such an account's folders are its labels and archiving moves to its Archive folder. New mail arrives at once where the server can say so (IMAP IDLE), and only what changed is asked for (CONDSTORE).

Outlook.com, Hotmail and Microsoft 365 sign in with Microsoft in the browser (**Continue with Microsoft**). That needs `NIMBRA_MICROSOFT_CLIENT_ID` (an app registered with Microsoft) in `~/.config/nimbra/oauth.env`; the sign-in is kept in the keychain, and IMAP and SMTP use it (XOAUTH2).

## Several accounts

Switch from the Accounts list in the sidebar or in Settings › Accounts. Each account has its own sign-in in the keychain and its own encrypted store and key; Sign out removes only that account. One account is the default (Settings › Accounts › Make default): its profile is at the bottom of the sidebar and new messages are from it. Drag an account by its handle in Settings › Accounts to change the order the sidebar and From list them in.

With two or more accounts the sidebar adds **All inboxes**: every account's inbox merged, newest first, loading further as you scroll. Every account is checked for new mail and notifies, snoozes and scheduled sends run for all of them, and the Dock badge counts them all. In All inboxes, actions and replies go to the message's own account; labels stay per account.
