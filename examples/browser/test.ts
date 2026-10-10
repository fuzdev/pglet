#!/usr/bin/env -S deno run --allow-read

// Automated test for pglet WASM
// This script loads and tests the WASM module programmatically

// Resolve paths relative to this file's location
const wasmPath = new URL("../../crates/pglet_wasm/pkg/pglet_wasm.js", import.meta.url);
const wasmBinaryPath = new URL("../../crates/pglet_wasm/pkg/pglet_wasm_bg.wasm", import.meta.url);

const { default: init, PgletBtree } = await import(wasmPath.href);

async function main() {
    console.log("pglet WASM Test");
    console.log("=".repeat(50));

    try {
        // Initialize WASM
        console.log("\n1. Initializing WASM module...");
        await init();
        console.log("   ✓ WASM module loaded");

        // Create database instance
        console.log("\n2. Creating database instance...");
        const db = new PgletBtree();
        console.log("   ✓ Database instance created");

        // Test CREATE TABLE
        console.log("\n3. Testing CREATE TABLE...");
        db.exec("CREATE TABLE users (id INT, name TEXT)");
        console.log("   ✓ Table created");

        // Test INSERT
        console.log("\n4. Testing INSERT...");
        db.exec("INSERT INTO users VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Carol')");
        console.log("   ✓ Rows inserted");

        // Test SELECT
        console.log("\n5. Testing SELECT...");
        const result1 = db.query("SELECT * FROM users ORDER BY id");
        console.log(`   ✓ Query returned ${result1.rowCount} rows`);
        console.log("   Columns:", result1.fields.map((f: any) => f.name).join(", "));
        console.log("   Rows:");
        for (const row of result1.rows) {
            console.log(`      id=${row.id}, name=${row.name}`);
        }

        // Test WHERE clause
        console.log("\n6. Testing SELECT with WHERE...");
        const result2 = db.query("SELECT name FROM users WHERE id > 1 ORDER BY name");
        console.log(`   ✓ Query returned ${result2.rowCount} rows`);
        for (const row of result2.rows) {
            console.log(`      name=${row.name}`);
        }

        // Test UPDATE
        console.log("\n7. Testing UPDATE...");
        db.exec("UPDATE users SET name = 'Alice Updated' WHERE id = 1");
        const result3 = db.query("SELECT * FROM users WHERE id = 1");
        console.log(`   ✓ Updated row: id=${result3.rows[0].id}, name=${result3.rows[0].name}`);

        // Test DELETE
        console.log("\n8. Testing DELETE...");
        db.exec("DELETE FROM users WHERE id = 2");
        const result4 = db.query("SELECT COUNT(*) as count FROM users");
        console.log(`   ✓ Remaining rows: ${result4.rows[0].count}`);

        // Test aggregates
        console.log("\n9. Testing aggregates...");
        db.exec("CREATE TABLE products (id INT, name TEXT, price FLOAT)");
        db.exec("INSERT INTO products VALUES (1, 'Laptop', 999.99), (2, 'Mouse', 29.99), (3, 'Keyboard', 79.99)");
        const result5 = db.query("SELECT COUNT(*) as count, AVG(price) as avg_price FROM products");
        console.log(`   ✓ Count: ${result5.rows[0].count}, Avg Price: ${result5.rows[0].avg_price}`);

        // Test ORDER BY and LIMIT
        console.log("\n10. Testing ORDER BY and LIMIT...");
        const result6 = db.query("SELECT name, price FROM products ORDER BY price DESC LIMIT 2");
        console.log(`   ✓ Top 2 products by price:`);
        for (const row of result6.rows) {
            console.log(`      ${row.name}: $${row.price}`);
        }

        console.log("\n" + "=".repeat(50));
        console.log("All tests passed! ✓");
        console.log("\nWASM binary size:", (await Deno.stat(wasmBinaryPath)).size, "bytes");

    } catch (error) {
        console.error("\n✗ Test failed:", error);
        Deno.exit(1);
    }
}

main();
