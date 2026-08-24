# Separate plugin declarations and values

Setting Declarations are immutable installation artifacts in `assets/config.json`, while Stored Setting Values are plugin-global runtime data in `data/<namespace>/<name>/store.json` and Secret plaintext is kept in the operating system credential store. This separation lets declarations change with plugin versions while user values survive upgrades without making mutable data part of a verified installation artifact or the application database.
