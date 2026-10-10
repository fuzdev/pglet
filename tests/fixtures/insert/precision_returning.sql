-- `RETURNING` reports the **stored** value, so it carries the rounding rather than
-- echoing the literal back.
-- @setup
CREATE TABLE tpr (id int, a timestamp(3), c interval(3));
-- @test
INSERT INTO tpr VALUES (1, '2020-01-02 03:04:05.6789', '1.23456 secs')
RETURNING id, a, c;
