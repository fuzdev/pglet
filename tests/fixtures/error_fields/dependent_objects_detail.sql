-- 2BP01 is three fields, not one sentence: the refusal in `M`, the dependent in `D`,
-- and the CASCADE suggestion in `H` — with the identifiers **unquoted**, which is the
-- one place pglet drops its own quoting convention to match the oracle.
-- @setup
CREATE TABLE p_dep (id INT PRIMARY KEY);
CREATE TABLE c_dep (a INT REFERENCES p_dep);

-- @test
-- @expect-error: a foreign key still depends on this table
-- @no-profile-lock: the 2BP01 comes from `executor/dependencies.rs`'s catalog-only edge resolver on a DDL statement; that module carries no `index_acceleration` cfg and reads no index
-- @expect-message
DROP TABLE p_dep;
