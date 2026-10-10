-- A partial index's membership is the predicate over the row, evaluated by the same walk,
-- so a row the left operand admits is indexed without evaluating the division — on both
-- build profiles, the query below reading the index where acceleration is on.
-- @profile-lock
-- @setup
CREATE TABLE sp (id INT, s TEXT);
CREATE INDEX sp_part ON sp (id) WHERE (id = 1 OR 1/(id-1) = 0);
INSERT INTO sp VALUES (1, 'x'), (2, 'y');
-- @test
SELECT id, s FROM sp WHERE (id = 1 OR 1/(id-1) = 0) AND id = 1;
