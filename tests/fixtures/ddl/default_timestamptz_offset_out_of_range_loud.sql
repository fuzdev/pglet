-- A column `DEFAULT` literal is read by the column type's input function when the table
-- is created, so an offset 18.6 refuses is refused there, not accepted and then raised
-- by every write that omits the column.
-- @expect-error: time zone displacement out of range
-- @expect-message
-- @test
CREATE TABLE ev (id INT, at TIMESTAMPTZ DEFAULT '2000-01-01 00:00:00+16');
