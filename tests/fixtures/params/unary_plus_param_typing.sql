-- A bound `$N` under unary `+` types as **float8**, from the operator.
--
-- Postgres has no polymorphic unary `+`, so resolving `+ unknown` picks the float8
-- form — the same resolution that makes `SELECT +'abc'` fail `22P02 invalid input
-- syntax for type double precision`. The sibling column never gets a say: even in
-- `WHERE int_col = +$1`, Postgres reports `$1` as float8, not int4. Binding it as
-- int4 is a client-side serialization failure on both engines, which is why this
-- fixture binds float8.
--
-- pglet's `param_types` probes all pattern-match a **bare** `Expr::Parameter`. Once
-- the parser began keeping the `+` node (`Expr::UnaryPlus`) instead of discarding it,
-- `+$1` matched none of them and reported OID 0 — the unknown-type report that sends
-- a binary-protocol client (tokio-postgres) into a type-introspection loop.
-- @setup
CREATE TABLE upp (x INT, label TEXT);
INSERT INTO upp VALUES (1, 'one'), (2, 'two'), (3, 'three');
-- @param $1 float8 2
-- @test
SELECT x, label FROM upp WHERE x = +$1;
