-- The first proposal of the key **inserts** the row, and the second meets that row: 18.6
-- refuses it `21000` as it refuses a row the first proposal updated. A refusal that fires
-- only for a row the statement *updated* is red here; its twin is
-- `on_conflict_do_update_same_row_twice_loud.sql`.
-- @profile-lock
-- @setup
CREATE TABLE ocsi (id INT PRIMARY KEY, a INT, b INT);

-- @test
-- @expect-error: ON CONFLICT DO UPDATE command cannot affect row a second time (21000)
INSERT INTO ocsi VALUES (1, 5, 0), (1, 0, 0)
ON CONFLICT (id) DO UPDATE SET a = ocsi.a + 1, b = ocsi.a
RETURNING *;
