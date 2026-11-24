/// Tells whether this process created the database it opened.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DatabaseOrigin {
    /// No migration had ever been applied, so this process initialized the database.
    Created,
    /// An earlier process already applied migrations.
    Existing,
}
