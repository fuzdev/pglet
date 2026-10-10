-- A bound `$N` filter on a VIRTUAL-table column must be typed. Shape: the spine's
-- `auth_tables_exist` probe — `WHERE table_name = $1` over `information_schema.tables`.
-- The column lives on a synthesized catalog view (not `Schema.tables`), so parameter
-- inference used to leave `$1` untyped (OID 0); a binary-protocol client
-- (tokio-postgres, this harness) then rejects the bind in `ParameterDescription`.
-- `$1` must type as text via the virtual table's column list, and the EXISTS match
-- the live catalog. (Regression guard for the `VirtualScan` param-typing path; see
-- `param_types.rs` `column_resolved_in_relation`.)
-- @setup
CREATE TABLE account (id INT, name TEXT);
-- @param $1 text 'account'
-- @test
SELECT EXISTS (
  SELECT 1 FROM information_schema.tables
  WHERE table_schema = 'public' AND table_name = $1
) AS present;
