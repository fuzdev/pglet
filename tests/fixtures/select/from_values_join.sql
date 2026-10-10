-- @setup

-- @test
-- a VALUES derived table composes in a JOIN (on both sides), with WHERE and ORDER BY
SELECT k.label, v.val
FROM (VALUES (1, 'one'), (2, 'two'), (3, 'three')) AS k(id, label)
JOIN (VALUES (1, 'A'), (2, 'B')) AS v(kk, val) ON k.id = v.kk
WHERE v.kk >= 1
ORDER BY k.id;
