-- The build's duplicate comes before the NOT NULL scan too: a NULL is distinct in the build,
-- so the two `x` rows are its `23505`, ahead of the NULL row's `23502`.
-- @setup
CREATE TABLE t (a text);
INSERT INTO t VALUES (NULL), ('x'), ('x');
-- @test
-- @expect-error: the duplicate before the NULL
-- @expect-message
-- @profile-lock
ALTER TABLE t ADD PRIMARY KEY (a);
