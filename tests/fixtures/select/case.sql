-- @setup
CREATE TABLE sessions (id INT, kind TEXT, score INT);
INSERT INTO sessions VALUES (1, 'web', 5), (2, 'api', 50), (3, 'web', NULL), (4, 'cli', 0);

-- @test
-- Searched `CASE` (a boolean per WHEN) and simple `CASE` (operand compared by
-- equality), including an absent ELSE (→ NULL) — the spine's conditional update
-- shape (`CASE WHEN … THEN … ELSE … END`).
SELECT id,
       CASE WHEN score IS NULL THEN 'unknown'
            WHEN score >= 50 THEN 'high'
            ELSE 'low' END AS bucket,
       CASE kind WHEN 'web' THEN 1 WHEN 'api' THEN 2 END AS kind_code
FROM sessions
ORDER BY id ASC;
