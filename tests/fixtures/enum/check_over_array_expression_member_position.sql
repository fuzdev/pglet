-- A `CHECK` over an array expression — the stored-expression write path, re-evaluated on
-- every row, where the member order decides which rows land and the label bytes admit
-- exactly the opposite set with no error on the row they take.
--
-- The setup inserts the row the member order **keeps**; a label-byte engine refuses it
-- here, so the divergence surfaces as a failing setup rather than as a silent swap.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE c (id INT, m mood NOT NULL, CHECK (ARRAY[m] < ARRAY['ok'::mood]));
INSERT INTO c VALUES (1, 'sad');

-- @test
SELECT id, m::text AS label FROM c ORDER BY id;
