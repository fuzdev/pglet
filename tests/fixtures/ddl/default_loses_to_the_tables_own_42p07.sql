-- A default is cooked once the heap exists (`AddRelationNewConstraints`), so an
-- already-existing table of this statement's name outranks it.
-- @setup
CREATE TABLE def_taken (q INT);
-- @expect-error: the table's own 42P07 outranks the bad default
-- @test
CREATE TABLE def_taken (a INT, b INT DEFAULT 'x');
