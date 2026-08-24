# Model plugin status as orthogonal axes

Ora represents Installation Validity, Durable Eligibility, Configuration Summary, and Runtime Status independently in plugin snapshots. Configuration Summary is itself exclusive: Not Declared, Available with Complete or Incomplete Configuration Completeness, or Unavailable with an error code. A combined plugin state enum would grow combinatorially, while independent availability and completeness booleans would permit contradictory configuration states.
