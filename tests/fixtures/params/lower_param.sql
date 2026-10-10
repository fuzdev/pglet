-- A bound param inside LOWER() on one side of a comparison — the spine's
-- case-insensitive account lookup (`LOWER(username) = LOWER($1)`,
-- query_account_by_username). The param must type as text; left untyped (OID 0) a
-- binary-protocol client (tokio-postgres) loops on type introspection.
-- @setup
CREATE TABLE account (id INT, username TEXT);
INSERT INTO account VALUES (1, 'Keeper'), (2, 'Other');
-- @param $1 text keeper
-- @test
SELECT id, username FROM account WHERE LOWER(username) = LOWER($1);
