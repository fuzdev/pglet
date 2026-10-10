-- A primary-key swap, then a read through the key: the index and the row agree on the
-- swapped values. The in-place reading stored `(10, 10)`.
-- @profile-lock
-- @setup
CREATE TABLE sswp (k INT PRIMARY KEY, k2 INT);
INSERT INTO sswp VALUES (1, 10), (2, 20);
UPDATE sswp SET k = k2, k2 = k;

-- @test
SELECT k, k2 FROM sswp WHERE k = 10;
