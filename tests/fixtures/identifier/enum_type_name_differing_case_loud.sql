-- The mirror polarity of `enum_type_name_folds_ascii_only.sql`, and the silent one: a
-- differently-cased spelling of a non-ASCII enum name must not resolve. The parser folds
-- ASCII only, so `CREATE TYPE ZÄP` registers `zÄp` and the later `zäp` names nothing —
-- `42704` on both engines. Under a Unicode `to_lowercase()` at
-- `executor::ddl::execute_create_type` the key is `zäp`, the column resolves, and the
-- table and its rows are accepted with no error anywhere to notice.
-- Both statements are unquoted ordinary DDL.
-- @setup
CREATE TYPE ZÄP AS ENUM ('a');

-- @test
-- @expect-error: `zäp` is not the name `ZÄP` registered, which folds ASCII-only to `zÄp` (42704)
CREATE TABLE zt (c zäp);
