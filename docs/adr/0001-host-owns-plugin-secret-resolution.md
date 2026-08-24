# Host owns plugin Secret resolution

Secrets are outside the first Plugin Configuration delivery. When Secret support is introduced, Ora alone will resolve them and inject their plaintext into explicitly declared inputs of a Managed Agent Process. Agent Plugins will receive opaque references rather than plaintext, because giving a privileged plugin process decrypted credentials would defeat the isolation provided by the credential store and prevent Ora from enforcing least-privilege delivery.
