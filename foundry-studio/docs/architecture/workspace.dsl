workspace "GTP Queue Simulator" "C4 architecture for the GTP Queue Simulator" {

    model {
        engineer = person "Warehouse Engineer" "Configures scenarios and analyzes simulation results"

        simulator = softwareSystem "GTP Queue Simulator" "Discrete event simulator for warehouse goods-to-person operations using Desmo-J" {
            envoy = container "Envoy Gateway" "Routes API and static requests" "Envoy 1.28"
            nginx = container "nginx" "Serves SPA static files" "nginx:alpine"
            frontend = container "Frontend SPA" "Warehouse visualization and simulation config" "React 19, TypeScript, Vite" "Browser"
            fleetSim = container "Fleet Simulator" "Simulation engine and REST API" "Spring Boot 3.2, Java 21" {
                // REST API
                simController = component "SimulationController" "POST /api/simulation/run" "Spring MVC"

                // Orchestration
                sim = component "Simulator" "Top-level orchestrator: build warehouse, run engine, collect results" "Spring Bean"
                whBuilder = component "WarehouseBuilder" "4-phase warehouse construction from config + gRPC services" "Java"
                engine = component "DesmojSimulationEngine" "Initializes orders, sets up experiment, runs Desmo-J, collects results" "Desmo-J"
                model = component "DesmojWarehouseModel" "Desmo-J Model: operator resources, process scheduling, findPath()" "Desmo-J Model"

                // Robot Processes (SimProcess)
                rtpProcess = component "DesmojRtpRobotProcess" "RTP robot lifecycle: pick up rack, visit stations, return" "Desmo-J SimProcess"
                vtmProcess = component "DesmojVtmProcess" "VTM lifecycle: priority-based tote transport rack to relay" "Desmo-J SimProcess"
                htmProcess = component "DesmojHtmProcess" "HTM lifecycle: tote transport relay to station" "Desmo-J SimProcess"

                // Background Processes
                binMgmt = component "DesmojBinManagementProcess" "Selects orders and assigns tasks to racks/totes every 5s" "Desmo-J SimProcess"
                metricMon = component "DesmojMetricMonitoringProcess" "Logs utilization, checks order completion every 10s" "Desmo-J SimProcess"
                snapCapture = component "DesmojSnapshotCaptureProcess" "Captures keyframe/delta snapshots for animation" "Desmo-J SimProcess"

                // Domain
                orderMgr = component "OrderManager" "Order lifecycle: pending, binned, completed" "Java"
                orderStrategy = component "OrderSelectionStrategy" "Pluggable: PPF, Tiering, or TPQ algorithm" "Strategy Pattern"
                binManager = component "BinManager" "Operator workload: bin states (available, processing, completed)" "Java"
                stationHelper = component "DesmojStationHelper" "Processes picks at station, batched time advancement" "Java"
                fleetManager = component "FleetManager" "Robot fleet tracking and assignment" "Java"
                storageSpaceMgr = component "StorageSpaceManager" "Unified access to racks and totes" "Java"
                inventory = component "WarehouseInventory" "Item quantities per storage space, reserve/consume" "Java"
                scenarioMgr = component "ScenarioManager" "Loads order CSV and item catalog from scenarios" "Java"

                // Internal relationships
                simController -> sim "Delegates"
                sim -> whBuilder "Builds warehouse"
                sim -> engine "Runs simulation"
                engine -> model "Creates and configures"
                model -> rtpProcess "Schedules (RTP)" "Desmo-J"
                model -> vtmProcess "Schedules (RELAY)" "Desmo-J"
                model -> htmProcess "Schedules (RELAY)" "Desmo-J"
                model -> binMgmt "Schedules" "Desmo-J"
                model -> metricMon "Schedules" "Desmo-J"
                model -> snapCapture "Schedules" "Desmo-J"
                binMgmt -> orderMgr "processOrders()"
                orderMgr -> orderStrategy "selectOrders()"
                orderMgr -> binManager "assignOrderToBin()"
                orderMgr -> inventory "reserveStock()"
                rtpProcess -> model "findPath() and rack lookup" "Desmo-J"
                rtpProcess -> stationHelper "processStorageSpaceTasks()"
                vtmProcess -> model "findPath() and tote lookup" "Desmo-J"
                htmProcess -> model "findPath() and relay lookup" "Desmo-J"
                htmProcess -> stationHelper "processStorageSpaceTasks()"
                stationHelper -> inventory "Consumes inventory"
                whBuilder -> scenarioMgr "Loads item catalog + orders"
                whBuilder -> storageSpaceMgr "Creates"
                whBuilder -> inventory "Populates"
                whBuilder -> fleetManager "Creates robot fleet"
            }
            layoutSvc = container "Layout Service" "Warehouse layout generation" "Spring Boot 3.2, Java 21"
            navSvc = container "Navigation Service" "A* robot pathfinding" "Spring Boot 3.2, Java 21"
            putSvc = container "Put Service" "Storage algorithms and inventory init" "Spring Boot 3.2, Java 21"
        }

        cloudRun = softwareSystem "Google Cloud Run" "Hosts the simulator as a multi-container service" "External"

        // System-level relationships (L1 Context)
        engineer -> simulator "Configures scenarios and runs simulations" "HTTPS"
        simulator -> cloudRun "Deployed to" "us-central1"

        // Container-level relationships (L2)
        engineer -> envoy "Uses" "HTTPS :8080"
        envoy -> nginx "Serves frontend" "HTTP :8090"
        envoy -> fleetSim "Routes /api/*" "HTTP :8085"
        envoy -> layoutSvc "Routes /api/warehouse-map/*" "HTTP :8082"
        nginx -> frontend "Delivers" "Static files"
        fleetSim -> layoutSvc "Gets warehouse layout" "gRPC :9091"
        fleetSim -> navSvc "Finds robot paths" "gRPC :9092"
        fleetSim -> putSvc "Initializes inventory" "gRPC :9090"
        layoutSvc -> putSvc "Calculates bin popularity" "gRPC :9090"

        // Component-level relationships (L3 — fleet-simulator to external services)
        whBuilder -> layoutSvc "GetLayout" "gRPC :9091"
        whBuilder -> putSvc "InitializeInventory" "gRPC :9090"
        engine -> navSvc "CreateSession / DeleteSession" "gRPC :9092"
        model -> navSvc "FindPath (per robot move)" "gRPC :9092"

        // Deployment (L4)
        deploymentEnvironment "Production" {
            deploymentNode "Engineer's Browser" "" "Chrome / Firefox" {
                containerInstance frontend
            }
            deploymentNode "Google Cloud Platform" "" "us-central1" {
                deploymentNode "Cloud Run" "greymatter-des-simulator" "concurrency=1, min/max=1, timeout=3600s" {
                    deploymentNode "Envoy Sidecar" "" "512Mi, 0.5 CPU" {
                        containerInstance envoy
                    }
                    deploymentNode "nginx Sidecar" "" "256Mi, 0.5 CPU" {
                        containerInstance nginx
                    }
                    deploymentNode "Fleet Simulator" "" "8Gi, 4 CPU" {
                        containerInstance fleetSim
                    }
                    deploymentNode "Layout Service" "" "4Gi, 1 CPU" {
                        containerInstance layoutSvc
                    }
                    deploymentNode "Navigation Service" "" "4Gi, 1 CPU" {
                        containerInstance navSvc
                    }
                    deploymentNode "Put Service" "" "4Gi, 1 CPU" {
                        containerInstance putSvc
                    }
                }
            }
        }
    }

    views {
        systemContext simulator "SystemContext" "System Context — who uses it, what it is" {
            include *
            autoLayout
        }

        container simulator "Containers" "All services, protocols, and ports" {
            include *
            autoLayout
        }

        dynamic simulator "SimulationRun" "Run Simulation — the core workflow" {
            engineer -> envoy "POST /api/simulation/run (JSON config)"
            envoy -> fleetSim "Forward request"
            fleetSim -> layoutSvc "GetLayout (gRPC)"
            fleetSim -> putSvc "InitializeInventory (gRPC)"
            fleetSim -> navSvc "CreateSession — sends map (gRPC)"
            fleetSim -> navSvc "FindPath — repeated per robot move (gRPC)"
            fleetSim -> navSvc "DeleteSession (gRPC)"
            fleetSim -> envoy "Return SimulationResult (JSON)"
            envoy -> engineer "Results + metrics"
            autoLayout
        }

        component fleetSim "FleetSimComponents" "Fleet Simulator — internal components" {
            include *
            autoLayout
        }

        dynamic fleetSim "RtpCycle" "RTP Robot Pick Cycle" {
            binMgmt -> orderMgr "Select orders (every 5s)"
            orderMgr -> orderStrategy "Score and rank orders"
            orderMgr -> inventory "Reserve stock"
            orderMgr -> binManager "Assign order to bin"
            rtpProcess -> model "Find rack with tasks"
            rtpProcess -> model "findPath() to rack"
            model -> navSvc "FindPath (gRPC)"
            rtpProcess -> stationHelper "Process picks at station"
            stationHelper -> inventory "Consume inventory"
            rtpProcess -> model "findPath() to storage"
            autoLayout
        }

        dynamic fleetSim "RelayCycle" "Relay System — VTM + HTM Cycle" {
            binMgmt -> orderMgr "Select orders (every 5s)"
            orderMgr -> orderStrategy "Score and rank orders"
            orderMgr -> inventory "Reserve stock"
            vtmProcess -> model "Find tote in zone"
            vtmProcess -> model "findPath() to rack"
            model -> navSvc "FindPath (gRPC)"
            vtmProcess -> model "findPath() to relay"
            htmProcess -> model "Find relay with tote"
            htmProcess -> model "findPath() to relay"
            htmProcess -> stationHelper "Process picks at station"
            stationHelper -> inventory "Consume inventory"
            htmProcess -> model "findPath() to return relay"
            autoLayout
        }

        deployment simulator "Production" "Deployment" "Production deployment on Cloud Run (us-central1)" {
            include *
            autoLayout
        }

        styles {
            element "Person" {
                shape Person
            }
            element "Browser" {
                shape WebBrowser
            }
            element "External" {
                background #999999
                color #ffffff
            }
        }

        theme default
    }

}
