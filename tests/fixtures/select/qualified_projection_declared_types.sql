-- A qualified projection must declare the **source column's** type, not `text`.
-- Descriptor resolution matched a qualified `t.c` only against an input column
-- literally named `"t.c"` — which a join's merged columns are and a base-table
-- scan's are not — so `SELECT t.x FROM t` resolved nothing and fell back to
-- `text`. Over the simple query protocol every value is a string, so the bytes
-- still compared equal; only the `RowDescription` said otherwise, and that is what
-- psql, node-postgres, and the wasm adapter type their output by.
-- @setup
CREATE TABLE qpt (k INT PRIMARY KEY, x INT, f FLOAT8, b BOOL, t TEXT);
INSERT INTO qpt VALUES (1, 10, 1.5, true, 'hi');

-- @test
SELECT qpt.k, qpt.x, qpt.f, qpt.b, qpt.t FROM qpt;
