# Add Kampus to your own Claude or ChatGPT

Kampus lets the AI assistant you already use answer questions about your campus: trains, study rooms, library hours, courses. This page shows how to add it to your own Claude or ChatGPT account.

You only need **one thing from your school: the Kampus address**, which looks like `https://mcp.school.example/mcp`. You never need a client ID, a secret, a password for Kampus, or a token. If a page asks you for one of those, stop and ask your school's IT office.

## 1. Paste the address into your assistant

**Claude** (claude.ai): open the connector settings, choose **Add custom connector**, give it a name such as "Kampus", paste the Kampus address and add it. Leave any advanced or OAuth fields empty.

**ChatGPT**: turn on developer mode, then create a connector, paste the Kampus address and choose OAuth as the authentication. Leave the client ID and secret empty.

Plans change, but at the time of writing Claude's free plan allowed one custom connector, and ChatGPT's developer mode needed a paid plan.

## 2. Sign in with your school account

Your assistant shows a **Connect** button (or opens a browser window). It takes you to your school's Kampus sign-in page, which names your school. Choose the sign-in button; it names your school's sign-in service (for a Google Workspace school, `accounts.google.com`). Sign in there as you normally do.

Kampus never sees your password. It doesn't keep your name or email address either: your assistant gets a code name for you, plus your role (for example, student).

If you see **"No Kampus access for this account"**, your school hasn't given your kind of account access yet. Ask your IT office.

## 3. Allow the assistant

Kampus then asks whether to allow that assistant. The page names the assistant by its web address (for example `claude.ai` or `chatgpt.com`) or by a name your school chose, never by a name the assistant made up for itself. It also shows the Kampus address and any extra permission the assistant asked for, such as `rooms:book` (confirming study-room bookings).

Choose **Allow**. You go back to your assistant, connected. If you choose **Deny**, nothing is shared and the assistant isn't connected.

You won't be asked again on the same assistant unless it asks for more permissions (it then has two entries on the connections page) or you disconnect it. Your assistant renews its access in the background, so you shouldn't have to sign in every day.

## 4. Try a public question

Ask something about your campus, for example:

> When's the next train from Overbrook?

Public questions work for everyone, even before you sign in. Which tools answer depends on what your school has turned on; a tool your school doesn't run says so instead of guessing.

## 5. What a refusal looks like

Every Kampus tool shows up in your assistant, but your school decides who may use which one. Personal data, such as your coursework deadlines, is **off unless your school turns it on** for assistants you add yourself.

When the school says no, the assistant tells you the request isn't allowed (Kampus answers "permission denied"). That's Kampus working as intended, not an error. Nothing about you was fetched: Kampus refuses before it touches any data.

If you ask something that needs you to be signed in and you aren't, the assistant asks you to connect or reconnect instead.

Your school keeps a log of which tool was used, when, and whether it was allowed. The log never contains what you asked or what the answer was, and you appear in it only as a scrambled ID.

## 6. See and remove connected assistants

Open `/auth/connections` on your school's Kampus site (for example `https://mcp.school.example/auth/connections`) in the same browser you signed in with. It works for 8 hours after a sign-in; after that, connect from your assistant again to open it. The page lists every assistant you allowed, with its permissions and when you connected it.

Choose **Disconnect** next to an assistant to cut it off. It can no longer renew its access. Anything it already holds stops working within 15 minutes (or the time your school set), and your school's rules still apply to every request until then. To use Kampus there again, connect from the assistant and allow it again.

You can also remove the connector in your assistant's own settings.

## Good to know

- Times may come back in UTC rather than your local time. Assistants usually convert them, but check if a time looks off.
- If the assistant suddenly asks you to connect again, your school's Kampus server may have restarted. Connect again; it's safe.
- Questions about what data your school shares, and with which assistants, go to your school's IT office. They set the rules; Kampus enforces them.
