-- A `client_encoding` 18.6 does not know is 18.6's own `22023 invalid value for parameter
-- "client_encoding"`, raised before pglet's UTF-8-only refusal (`0A000`) is asked — that one
-- is reserved for an encoding 18.6 takes (`set_client_encoding_non_utf8.sql`). The names 18.6
-- knows are its `pg_encname_tbl`, read as its `clean_encoding_name` reads a name.
-- @test
-- @expect-error: invalid value for parameter "client_encoding"
SET client_encoding = 'NOSUCH';
