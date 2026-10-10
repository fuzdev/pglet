-- The child-side 23503: `t` is the child, and the DETAIL names the parent table.
-- @setup
CREATE TABLE p_fk (id INT PRIMARY KEY);
CREATE TABLE c_fk (a INT REFERENCES p_fk(id));

-- @test
-- @expect-error: no such parent row
-- @no-profile-lock: the 23503 is raised by `enforce_foreign_keys` → `parent_tuple_exists` on the INSERT's write path; that probe prefers the parent's covering unique index on **both** profiles (no `index_acceleration` cfg in `executor/constraints.rs`), and the INSERT plans no predicate
-- @expect-message
INSERT INTO c_fk VALUES (99);
