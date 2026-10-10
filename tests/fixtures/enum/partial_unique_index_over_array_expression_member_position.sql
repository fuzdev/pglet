-- The second write path over an array expression: a **unique partial** index whose
-- predicate orders one. Membership is decided in two objects nothing compares — the
-- executor's `PreparedIndex` and the storage layer's `IndexKeying` — so both must
-- read the array node's element enum or the duplicate lands silently.
--
-- Under the member order both rows satisfy `ARRAY[m] > ARRAY['sad'::mood]` and share
-- `k = 1`, so the second is `23505`; under the label bytes neither row is in the index.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE pa (id INT, k INT, m mood NOT NULL);
CREATE UNIQUE INDEX pa_u ON pa (k) WHERE ARRAY[m] > ARRAY['sad'::mood];
INSERT INTO pa VALUES (1, 1, 'ok');

-- @profile-lock
-- @test
-- @expect-error: both rows are in the partial index, so the duplicate k collides
INSERT INTO pa VALUES (2, 1, 'happy');
