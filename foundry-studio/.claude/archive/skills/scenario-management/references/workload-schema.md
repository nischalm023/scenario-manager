# Workload Scenario Schema

## ScenarioInfo Record

```java
public record ScenarioInfo(
    String id,                              // Unique identifier
    String name,                            // Human-readable name
    String description,                     // Distribution characteristics
    String createdDate,                     // ISO 8601 timestamp
    ItemCatalogInfo itemCatalog,            // SKU/inventory metadata
    OrderFileInfo orderFile                 // Order/pick metadata
)
```

---

## Directory Structure

```
scenarios/workloads/{id}/
├── scenario_info.json      # Metadata file
├── item_catalog.csv        # SKU data with inventory
└── orders.csv              # Order data with picks
```

---

## Complete JSON Schema

### scenario_info.json Example

```json
{
  "id": "scenario_a",
  "name": "Small Warehouse 1",
  "description": "500 SKUs and 1000 orders with 20% popular SKUs and 50% inventory share",
  "createdDate": "2025-11-10",
  "itemCatalog": {
    "filename": "item_catalog.csv",
    "totalSkus": 500,
    "popularSkus": 100,
    "categories": ["Electronics", "Sports", "Home", "Clothing", "Books"],
    "popularSkusPercentage": 20.0,
    "inventoryPopularQuantityShare": 50.0,
    "totalInventoryUnits": 20000
  },
  "orderFile": {
    "filename": "orders.csv",
    "totalOrders": 1000,
    "totalPicks": 4000,
    "orderPopularPicksShare": 80.0
  }
}
```

### Large-Scale Example

```json
{
  "id": "scenario_b",
  "name": "O&M Full Scale Infra | 0.3x Eaches UOM",
  "description": "4100 SKUs and 1000 orders with 9% popular SKUs and 65% inventory share",
  "createdDate": "2025-12-11",
  "itemCatalog": {
    "filename": "item_catalog.csv",
    "totalSkus": 4100,
    "popularSkus": 369,
    "categories": ["Electronics", "Books", "Clothing", "Home", "Sports"],
    "popularSkusPercentage": 9.0,
    "inventoryPopularQuantityShare": 65.0,
    "totalInventoryUnits": 54000
  },
  "orderFile": {
    "filename": "orders.csv",
    "totalOrders": 1000,
    "totalPicks": 13000,
    "orderPopularPicksShare": 65.0
  }
}
```

---

## ItemCatalogInfo Schema

```json
{
  "filename": "item_catalog.csv",           // CSV file name
  "totalSkus": 500,                         // Total unique products
  "popularSkus": 100,                       // High-velocity SKU count
  "categories": ["Electronics", "Sports"],  // Product categories
  "popularSkusPercentage": 20.0,            // % of SKUs marked popular (0-100)
  "inventoryPopularQuantityShare": 50.0,    // % inventory in popular SKUs (0-100)
  "totalInventoryUnits": 20000              // Total inventory quantity
}
```

---

## OrderFileInfo Schema

```json
{
  "filename": "orders.csv",                 // CSV file name
  "totalOrders": 1000,                      // Number of orders
  "totalPicks": 4000,                       // Total items to pick
  "orderPopularPicksShare": 80.0            // % picks from popular SKUs (0-100)
}
```

---

## CSV File Formats

### item_catalog.csv

```csv
sku,name,category,weight_kg,max_inventory_quantity,ordered_quantity,is_popular
SKU001,Product 1,Electronics,0.5,150,45,true
SKU002,Product 2,Sports,1.2,80,12,false
SKU003,Product 3,Home,0.8,200,78,true
```

**Columns:**
| Column | Type | Description |
|--------|------|-------------|
| `sku` | String | Unique SKU identifier |
| `name` | String | Product name |
| `category` | String | Product category |
| `weight_kg` | Double | Product weight in kg |
| `max_inventory_quantity` | Integer | Max inventory units for this SKU |
| `ordered_quantity` | Integer | Historical ordered quantity (popularity) |
| `is_popular` | Boolean | Whether SKU is marked as popular |

### orders.csv

```csv
order_id,customer_type,priority,sku,quantity
ORD001,premium,high,SKU001,2
ORD001,premium,high,SKU003,1
ORD002,standard,medium,SKU002,3
ORD003,bulk,low,SKU001,5
```

**Columns:**
| Column | Type | Description |
|--------|------|-------------|
| `order_id` | String | Order identifier (groups related picks) |
| `customer_type` | String | `premium`, `standard`, `bulk` |
| `priority` | String | `low`, `medium`, `high`, `urgent` |
| `sku` | String | SKU to pick |
| `quantity` | Integer | Quantity to pick |

---

## Generation Parameters

When creating a new workload via POST `/api/scenarios/workloads/create`:

```json
{
  "name": "My Workload",
  "description": "Custom workload description",
  "numSkus": 500,
  "totalInventoryUnits": 20000,
  "numOrders": 1000,
  "totalOrderPicks": 4000,
  "popularSkusPercentage": 20.0,
  "inventoryPopularQuantityShare": 50.0,
  "orderPopularPicksShare": 80.0,
  "categories": ["Electronics", "Sports", "Home", "Clothing", "Books"]
}
```

### Parameter Ranges

| Parameter | Range | Default | Description |
|-----------|-------|---------|-------------|
| `numSkus` | 10-10000 | 500 | Total SKU count |
| `totalInventoryUnits` | 100-1000000 | 20000 | Total inventory |
| `numOrders` | 10-10000 | 1000 | Order count |
| `totalOrderPicks` | 10-100000 | 4000 | Total picks |
| `popularSkusPercentage` | 1-99 | 20 | % SKUs marked popular |
| `inventoryPopularQuantityShare` | 1-99 | 50 | % inventory in popular |
| `orderPopularPicksShare` | 1-99 | 80 | % picks from popular |

---

## Pareto Distribution

The workload generator uses **Pareto distribution** (80/20 rule):
- Popular SKUs (typically 20%) receive disproportionate inventory and picks
- `popularSkusPercentage` controls how many SKUs are marked popular
- `inventoryPopularQuantityShare` controls what % of inventory goes to popular SKUs
- `orderPopularPicksShare` controls what % of picks come from popular SKUs

**Typical configurations:**

| Scenario | Popular % | Inventory Share | Pick Share |
|----------|-----------|-----------------|------------|
| Standard Pareto | 20% | 50% | 80% |
| Heavy concentration | 10% | 65% | 85% |
| Flat distribution | 40% | 45% | 55% |

---

## Item Record (Domain Model)

```java
public record Item(
    String sku,
    String name,
    String category,
    double weight,
    double volume,
    double pickTimeMultiplier,
    int maxInventoryQuantity,
    int pastOrderedQuantity          // Popularity indicator
)
```

---

## Analysis Endpoints

| Endpoint | Purpose |
|----------|---------|
| `GET /api/scenarios/workloads/{id}/sku-data` | SKU pick quantities and popularity |
| `GET /api/scenarios/workloads/{id}/order-analysis` | Order complexity, avg picks |
| `GET /api/scenarios/workloads/statistics` | Aggregate stats across all workloads |

---

## File Location

`apps/fleet-simulator/src/main/resources/scenarios/workloads/{id}/`

## Related Classes

- `ScenarioModels.ScenarioInfo` - Metadata record
- `ScenarioModels.ItemCatalogInfo` - Catalog metadata
- `ScenarioModels.OrderFileInfo` - Order metadata
- `ScenarioManager` - CRUD + generation
- `ScenarioGenerator` - Pareto-distributed generation
- `DataHandler` - Analysis and parsing
- `ItemCatalog` - Runtime catalog loading
