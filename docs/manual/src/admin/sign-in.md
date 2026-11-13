---
title: Single sign-on and provisioning
nav: Sign-in and provisioning
summary: Letting people sign in with your organization's accounts (OpenID Connect or SAML 2.0) and letting your identity provider create and remove accounts (SCIM).
---

Gantry supports two single sign-on protocols. Use the one your identity provider and your organization prefer; you can have several providers of either kind at once, each with its own button on the sign-in page.

| | OpenID Connect | SAML 2.0 |
| --- | --- | --- |
| Choose it when | Your provider offers it. It is the simpler of the two to set up and to rotate keys for. | Your organization standardizes on SAML, or the provider offers nothing else (ADFS, Shibboleth). |
| Set up under | **Sign-in**, **Add OpenID Connect provider** | **Sign-in**, **Add SAML provider** |

Sign-in providers need the *admin.security* permission, which system administrators have.

## Single sign-on with OpenID Connect

Gantry works with any identity provider that implements OpenID Connect: Microsoft Entra ID, Okta, Keycloak, Ping, Google Workspace, Auth0 and others. Your provider keeps doing what it does today, including multi-factor and conditional access; Gantry receives only the outcome.

### 1. Register Gantry at the provider

Create an application of the *web* type (confidential client, authorization code flow). You will need to enter a **redirect URI**; Gantry shows the exact value after step 2, in the form:

```
https://gantry.example.org/api/v1/auth/oidc/PROVIDER-ID/callback
```

Request the scopes `openid email profile`. Note the **issuer URL**, the **client ID** and the **client secret**.

| Provider | Issuer URL looks like |
| --- | --- |
| Microsoft Entra ID | `https://login.microsoftonline.com/TENANT-ID/v2.0` |
| Okta | `https://YOUR-ORG.okta.com` or a custom authorization server |
| Keycloak | `https://sso.example.org/realms/REALM` |
| Google Workspace | `https://accounts.google.com` |

### 2. Add the provider in Gantry

Under **Administration**, **Sign-in**, press **Add OpenID Connect provider** and fill in:

- **Button label**: what people see, for example *Entra ID*.
- **Issuer URL**, **Client ID**, **Client secret**: from step 1. The secret is stored encrypted and never shown again.
- **Allowed email domains**: optional. When set, only addresses in these domains may sign in through this provider.

Save. The provider's panel now shows its **redirect URI**; copy it into the application at the provider. Then press **Turn on**.

### 3. Try it

Open the sign-in page in a private browser window and use the new button. On first sign-in a person is matched to an existing Gantry account with the same verified email address, or a new account is created with the **Member** role.

### What Gantry checks

Every sign-in uses the authorization code flow with PKCE, a one-time state and nonce, and verifies the signature, issuer, audience and expiry of the identity token against the provider's published keys. Tokens signed with a shared secret are refused. An address the provider marks as unverified is never used to match or create an account.

### Turning off passwords

When single sign-on works for everyone, a system administrator can set `auth.allow_local_login = false` in the server configuration. Before doing so, make sure at least one system administrator can sign in through the provider. If you lock yourself out, the setting can be reverted in the configuration file.

## Single sign-on with SAML 2.0

Gantry acts as a SAML **service provider** and works with Microsoft Entra ID, ADFS, Okta, Ping, Keycloak, Shibboleth, Google Workspace and other standards-compliant identity providers.

### 1. Add the provider in Gantry

Open **Administration**, **Sign-in**, **Add SAML provider**.

The quickest way is to let Gantry read the provider's **metadata**: enter the **Metadata address** (it must be an `https` address that the server can reach) or paste the metadata XML, and press **Read metadata**. The entity ID, the sign-on address and the signing certificates are filled in for you. Otherwise enter them by hand.

| Field | What to enter |
| --- | --- |
| **Button label** | People see *Continue with …* on the sign-in page. |
| **Identity provider entity ID** | The provider's identifier (its *issuer*), exactly as it publishes it. |
| **Sign-on address (HTTP-Redirect)** | The provider's single sign-on URL for the HTTP-Redirect binding. |
| **Signing certificates** | The provider's signing certificate in PEM form. Paste several, one after another, when the provider is rotating certificates: a response signed with any of them is accepted. |
| **Attribute holding the email address** | The name of the attribute that carries the email address. Leave empty to use the common attribute names, and after those the NameID when it is an email address. |
| **Attribute holding the display name** | Likewise for the name. Leave empty to use the common names, or first name and last name together. |
| **Allowed email domains** | Comma separated. Leave empty to allow any account the provider accepts. |
| **Create an account the first time someone signs in** | Clear this to admit only people who already have a Gantry account (created by hand or by SCIM). |

Press **Add provider**. Accounts created at first sign-in get the **Member** role. The provider is listed with three values that the identity provider needs.

### 2. Register Gantry at the identity provider

Create an application (a *relying party* or *service provider*) there, and enter the values shown in Gantry. Each has a copy button.

| Gantry shows | The provider calls it | Looks like |
| --- | --- | --- |
| **Reply URL (ACS)** | Assertion consumer service URL, reply URL, single sign-on URL | `https://gantry.example.org/api/v1/auth/saml/…/acs` |
| **Our entity ID (audience)** | Identifier, audience URI, SP entity ID | `https://gantry.example.org/api/v1/auth/saml/…/metadata` |
| **Our metadata** | Federation metadata: import this address and the two values above are read from it | The same address |

Settings at the provider:

- **Sign the assertion or the response** with RSA and SHA-256 (or SHA-384 or SHA-512). SHA-1 is refused.
- **Do not encrypt assertions.** Gantry does not accept encrypted assertions; the connection is already protected by HTTPS.
- **NameID**: any stable identifier. A persistent identifier is best; an email address works.
- Release the **email address** and a **display name** as attributes.
- Assign the people or groups who may use Gantry.

Then press **Turn on** in Gantry.

### 3. Try it

Open the sign-in page in a private browser window and use the new button. On first sign-in a person is matched to an existing Gantry account with the same email address, or a new account is created with the role you chose.

### What Gantry checks

Before anyone is signed in, the response must pass all of these:

- it is well-formed XML with no document type declaration, and of modest size;
- the assertion that is used is covered by a valid signature made with one of the certificates **you** configured. Certificates and keys inside the message are ignored;
- the signature covers exactly the element that carries it, and only data from inside that verified element is read. This defeats signature-wrapping attacks;
- the issuer is your provider, the audience is this installation, and the recipient and destination are this installation's reply URL;
- the assertion is within its validity period, allowing two minutes for clock differences;
- it answers a sign-in that **this server started** within the last ten minutes, and carries the one-time value that was sent with that request;
- the same assertion has not been used before.

A response that fails any check is refused, the person sees *Single sign-on did not complete*, and a security event records which check failed.

### What is not supported

| Not supported | What to do |
| --- | --- |
| Encrypted assertions | Turn assertion encryption off at the provider. |
| Signed authentication requests | Do not require them at the provider. |
| Single log-out | Sessions end by their own timeouts, by signing out in Gantry, or by an administrator's **Sign out everywhere**. Deactivating an account (by hand or through SCIM) ends its sessions at once. |
| Signing certificates with keys other than RSA, or RSA keys shorter than 2,048 bits | Use an RSA certificate of at least 2,048 bits. |
| Sign-in started at the identity provider's own portal | Start at Gantry's sign-in page, or add a bookmark tile at the provider that points to `https://gantry.example.org/login`. It can be allowed per provider through the API (`allow_idp_initiated` in the provider's settings); leave it off unless you need it, because a response that answers no request is easier to replay. |

AI agent accounts can never sign in through a provider.

### Certificate rotation

Each provider's card shows until when each of its signing certificates is valid, in amber from 30 days before the end. When the identity provider announces a new certificate:

1. Press **Change** on the provider's card and paste the new certificate after the old one in **Signing certificates** (or use **Read metadata** again, which reads every certificate the provider publishes). Up to six certificates can be trusted at once.
2. Let the identity provider switch to the new certificate.
3. Press **Change** again and remove the old one.

Sign-in keeps working throughout. **Change** also edits the button label, the addresses, the attribute names and the allowed domains. Every change is recorded in the audit log.

## Automatic provisioning with SCIM 2.0

With SCIM your identity provider creates accounts before people first sign in, updates their names, **deactivates them when they leave**, and maintains groups.

1. Under **Users and agents** create an agent, for example *Entra provisioning*, with the **Provisioning client** role.
2. Create a token for it allowed to *Read, change and administer*.
3. In the identity provider's provisioning settings enter:
   - **Tenant URL**: shown on the Sign-in page, `https://gantry.example.org/scim/v2`
   - **Secret token**: the agent's token
4. Map at least user name (the email address), display name and active. Assign the people and groups to provision, then start provisioning.

Deactivating a person at the provider ends their Gantry sessions and tokens immediately. Groups pushed by the provider appear under **Groups** and can be given roles like any other.

## Troubleshooting sign-in

| Symptom | Likely cause |
| --- | --- |
| The provider says the redirect URI does not match | The URI registered at the provider differs from the one shown in Gantry. Copy it again; check `http` against `https` and any trailing slash. This also happens when the server's public address is not configured: ask the system administrator to check `server.public_url`. |
| *Single sign-on did not complete* | Look at **Security events** for the entry at that time; its detail names the check that failed (domain not allowed, unverified email, signature, expired). |
| A person gets a second, empty account | Their email at the provider differs from their existing Gantry account. Correct the address on one side. |
| SAML: *the assertion is encrypted, which is not supported* | Turn assertion encryption off at the identity provider. |
| SAML: *the assertion was issued by a different identity provider* | The **Identity provider entity ID** in Gantry does not match the issuer the provider sends. Copy it from the provider's metadata. |
| SAML: *the assertion is not valid yet (check both clocks)* | The server's clock or the provider's is wrong by more than two minutes. Ask the system administrator to check time synchronization. |
| SAML: the signature is refused | The provider signs with a certificate that is not in **Signing certificates** (often after a rotation), or with SHA-1. |
| SAML: *the assertion was not issued for this sign-in endpoint* | The reply URL registered at the provider differs from the one Gantry shows, or the server's public address is not configured. |
| Provisioning reports 401 | The token was revoked or expired, or the agent lost the Provisioning client role. |
| Provisioning reports 403 | The token's scope does not include administer. |
