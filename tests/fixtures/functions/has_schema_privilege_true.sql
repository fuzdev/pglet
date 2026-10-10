-- pglet's one role holds every privilege, so a schema it has and a privilege string 18.6
-- parses answer `true`: either privilege, any case, whitespace (a tab included) trimmed
-- around each comma-separated item, the `WITH GRANT OPTION` forms, and the three schemas
-- pglet answers for. Strict.
-- @test
SELECT has_schema_privilege('public', 'USAGE') AS usage, has_schema_privilege('public', 'create') AS lower,
  has_schema_privilege('public', ' usage ') AS padded, has_schema_privilege('public', 'USAGE,CREATE') AS list,
  has_schema_privilege('public', 'Usage , Create With Grant Option') AS grant_option,
  has_schema_privilege('public', E'USAGE\t') AS tab,
  has_schema_privilege('pg_catalog', 'USAGE') AS pg_catalog,
  has_schema_privilege('information_schema', 'USAGE') AS information_schema,
  has_schema_privilege(NULL, 'USAGE') AS null_schema, has_schema_privilege('public', NULL) AS null_privilege;
