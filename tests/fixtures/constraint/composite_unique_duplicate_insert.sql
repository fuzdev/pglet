-- A table-level composite UNIQUE (a, b) rejects a full-tuple duplicate, while
-- tuples differing in one column coexist.
-- @profile-lock
-- @setup
CREATE TABLE uq_ab (a INT, b TEXT, UNIQUE (a, b));
INSERT INTO uq_ab VALUES (1, 'x'), (1, 'y'), (2, 'x');

-- @test
-- @expect-error: duplicate composite UNIQUE value (1, 'x')
INSERT INTO uq_ab VALUES (1, 'x');
