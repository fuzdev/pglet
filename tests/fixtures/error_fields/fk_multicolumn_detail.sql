-- A composite foreign key names and values every referencing column.
-- @setup
CREATE TABLE p_fk3 (x INT, y TEXT, PRIMARY KEY (x, y));
CREATE TABLE c_fk3 (x INT, y TEXT, FOREIGN KEY (x, y) REFERENCES p_fk3);

-- @test
-- @expect-error: no such composite parent row
-- @no-profile-lock: same write-path probe as `fk_insert_detail.sql`, with a composite key — `parent_tuple_exists` is ungated and the INSERT plans no predicate
-- @expect-message
INSERT INTO c_fk3 VALUES (1, 'nope');
