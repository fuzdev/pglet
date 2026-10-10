-- ON CONFLICT (pk) DO UPDATE … WHERE filters which conflicting rows update: a row
-- whose WHERE is false is left unchanged and NOT re-inserted (and not returned).
-- EXCLUDED (the would-be-inserted row) is referenced nested inside the WHERE and,
-- case-insensitively, as the SET source.
-- @setup
CREATE TABLE inv (id INT PRIMARY KEY, qty INT, note TEXT);
INSERT INTO inv (id, qty, note) VALUES (1, 10, 'a'), (2, 20, 'b');
-- @test
INSERT INTO inv (id, qty, note) VALUES (1, 5, 'lo'), (2, 99, 'hi')
ON CONFLICT (id) DO UPDATE SET qty = EXCLUDED.qty, note = EXCLUDED.note
WHERE inv.qty < EXCLUDED.qty
RETURNING id, qty, note;
