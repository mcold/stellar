import { ChangeDetectorRef, Component, OnInit, OnDestroy, TemplateRef, ViewChild, ViewEncapsulation } from '@angular/core';
import { ActivatedRoute } from '@angular/router';
import { Location } from '@angular/common'; // Import Location
import { NbToastrService, NbDialogService } from '@nebular/theme';
import { LocalDataSource } from 'angular2-smart-table';
import { Subject } from 'rxjs';
import { takeUntil } from 'rxjs/operators';
import { NodeService } from '../../../../@core/data/node.service';
import { ClusterContextService } from '../../../../@core/data/cluster-context.service';
import { Cluster } from '../../../../@core/data/cluster.service';
import { ErrorHandler } from '../../../../@core/utils/error-handler';
import { MetricThresholds, renderMetricBadge, parseStarRocksDuration } from '../../../../@core/utils/metric-badge';
import { renderLongText } from '../../../../@core/utils/text-truncate';
import { AuthService } from '../../../../@core/data/auth.service';
import { themeColor } from '../../../../@core/utils/theme-color';
import * as dagre from 'dagre';

@Component({
  standalone: false,
  selector: 'ngx-profile-queries',
  templateUrl: './profile-queries.component.html',
  styleUrls: ['./profile-queries.component.scss'],
  encapsulation: ViewEncapsulation.None,
})
export class ProfileQueriesComponent implements OnInit, OnDestroy {
  // Data sources
  profileSource: LocalDataSource = new LocalDataSource();
  
  // State
  clusterId: number;
  activeCluster: Cluster | null = null;
  loading = true;
  autoRefresh = false; // Default: disabled
  refreshInterval: any;
  selectedRefreshInterval: number | 'off' = 'off'; // Default: off (Grafana style)
  refreshIntervalOptions = [
    { value: 'off', label: 'Off' },
    { value: 3, label: '3s' },
    { value: 5, label: '5s' },
    { value: 10, label: '10s' },
    { value: 30, label: '30s' },
    { value: 60, label: '1m' },
  ];
  private destroy$ = new Subject<void>();
  private profileDurationThresholds: MetricThresholds = { warn: 120000, danger: 240000 }; // Will be updated dynamically

  // Profile dialog
  currentProfileDetail: string = '';
  currentQueryId: string = '';
  profileDetailLoading = false;
  @ViewChild('profileDetailDialog') profileDetailDialogTemplate: TemplateRef<any>;
  
  // DAG Analysis
  analysisLoading = false;
  analysisError: string = '';
  analysisData: any = null;
  topNodes: any[] = [];
  graphNodes: any[] = [];
  graphEdges: any[] = [];
  graphWidth = 800;
  graphHeight = 600;
  selectedNode: any = null;
  zoomLevel = 1; // Zoom level for DAG
  // Panning state
  isPanning = false;
  startX = 0;
  startY = 0;
  translateX = 0;
  translateY = 0;
  
  // LLM Analysis state (loaded async after DAG)
  llmAnalysisLoading = false;
  llmAnalysisError: string = '';
  llmAnalysisStartTime: number = 0;
  llmAnalysisElapsedTime: number = 0;
  
  // Window control state
  isFullscreen = false; // Default to normal layout, toggle for full screen
  
  // Right Panel State (default width expanded by 30%)
  rightPanelWidth = 500;
  isRightPanelCollapsed = false;
  isResizingRight = false;
  
  // Right panel section collapse states (Figure 1 style)
  isTop10Collapsed = false;
  isSummaryCollapsed = false;
  isDiagnosisCollapsed = false;
  isRootCauseCollapsed = false; // Root cause analysis section
  showMemoryView = false; // Toggle between Time and Memory view
  
  // Graph direction: 'BT' (bottom-to-top) or 'LR' (left-to-right)
  graphDirection: 'BT' | 'LR' = 'BT';
  
  // Data flow animation (enabled by default)
  isFlowAnimationEnabled = true;
  
  // Node detail view state (when a node is selected)
  selectedNodeTab: 'core' | 'detail' | 'pipeline' = 'core';
  
  // Core metrics by operator type
  private coreMetricsByType: { [key: string]: string[] } = {
    'SCAN': ['RawRowsRead', 'BytesRead', 'CompressedBytesRead', 'ScanTime', 'IOTime', 'Table', 'Rollup', 'TabletCount', 'PushdownPredicates', 'BitmapIndexFilterRows', 'BloomFilterFilterRows', 'RuntimeInFilterRows', 'RuntimeBloomFilterRows'],
    'JOIN': ['BuildRows', 'ProbeRows', 'HashTableMemoryUsage', 'RuntimeFilterNum', 'JoinType', 'DistributionMode', 'HashTableSize', 'ProbeTime', 'BuildTime'],
    'AGGREGATE': ['InputRowCount', 'OutputRowCount', 'HashTableMemoryUsage', 'AggFuncComputeTime', 'GroupByKeyCount', 'StreamingTime', 'HashTableIteratorTime'],
    'EXCHANGE': ['ShuffleRowsPerBuffer', 'NetworkTime', 'WaitTime', 'OverallThroughput', 'PeakBufferUsage'],
    'SORT': ['SortKeys', 'SortType', 'TopNLimit', 'SortTime', 'MergeTime'],
    'PROJECT': ['ExprComputeTime', 'CommonSubExprComputeTime'],
    'DEFAULT': ['OperatorTotalTime', 'PullRowNum', 'PushRowNum', 'PeakMemoryUsage']
  };
  
  private nodeRankMap: Map<string, number> = new Map(); // Node rank by time percentage
  objectKeys = Object.keys; // Helper for template

  // Metric descriptions for tooltips
  // Reference: https://docs.starrocks.io/zh/docs/best_practices/query_tuning/query_profile_operator_metrics/
  metricDescriptions: { [key: string]: string } = {
    // ============================================
    // 概要指标 (Summary Metrics)
    // ============================================
    'Total': 'Total time consumed by the query, including the duration of the planning, execution and analysis phases',
    'QueryState': 'Query state, possible values are Finished, Error and Running',
    'QueryId': 'Unique identifier of the query',
    'StartTime': 'Timestamp when the query started',
    'EndTime': 'Timestamp when the query finished',
    'QueryType': 'Type of the query',
    'StarRocksVersion': 'StarRocks version in use',
    'User': 'User who executed the query',
    'DefaultDb': 'Default database used by the query',
    'SqlStatement': 'Executed SQL statement',
    'Variables': 'Important variables used in the query',
    'NonDefaultSessionVariables': 'Non-default session variables used in the query',
    'CollectProfileTime': 'Time spent collecting the profile',
    'IsProfileAsync': 'Indicates whether profile collection is asynchronous',

    // ============================================
    // 执行概览指标 (Execution Overview Metrics)
    // ============================================
    'FrontendProfileMergeTime': 'Profile merge time on the FE',
    'QueryAllocatedMemoryUsage': 'Total memory allocated across nodes',
    'QueryDeallocatedMemoryUsage': 'Total memory deallocated across nodes',
    'QueryPeakMemoryUsagePerNode': 'Peak memory usage per node',
    'QuerySumMemoryUsage': 'Total peak memory usage across nodes',
    'QueryExecutionWallTime': 'Wall clock time of the execution',
    'ExecutionWallTime': 'Total time spent executing the query',
    'QueryCumulativeCpuTime': 'Total CPU time across nodes',
    'QueryCpuTime': 'Cumulative CPU time of the query; all concurrent processes are summed, so this metric can exceed the actual execution time',
    'QueryCumulativeOperatorTime': 'Total time spent in operators, used as the denominator of the operator time percentage',
    'OperatorTime': 'Cumulative execution time of all operators',
    'QueryCumulativeNetworkTime': 'Total network time of the exchange nodes',
    'QueryNetworkTime': 'Sum of the network transfer time of all exchange nodes',
    'QueryCumulativeScanTime': 'Total IO time of the scan nodes',
    'QueryScanTime': 'Sum of the scan time of all SCAN nodes',
    'QueryPeakScheduleTime': 'Peak pipeline scheduling time',
    'PeakScheduleTime': 'Peak pipeline scheduling wait time',
    'QuerySpillBytes': 'Data spilled to disk',
    'SpillBytes': 'Amount of data spilled to disk',
    'ResultDeliverTime': 'Time spent delivering the result',
    'MemoryUsage': 'Query memory usage',

    // ============================================
    // Fragment 指标
    // ============================================
    'InstanceNum': 'Number of fragment instances',
    'InstanceIds': 'IDs of all fragment instances',
    'BackendNum': 'Number of participating BEs',
    'BackendAddresses': 'BE addresses',
    'FragmentInstancePrepareTime': 'Duration of the fragment preparation phase',
    'InstanceAllocatedMemoryUsage': 'Total memory allocated by the instances',
    'InstanceDeallocatedMemoryUsage': 'Total memory deallocated by the instances',
    'InstancePeakMemoryUsage': 'Peak memory usage across the instances',
    'FragmentID': 'Fragment identifier',

    // ============================================
    // Pipeline 指标
    // ============================================
    'DriverTotalTime': 'Total time consumed by the driver. DriverTotalTime = ActiveTime + PendingTime + ScheduleTime',
    'ActiveTime': 'Driver execution time. ActiveTime = ∑OperatorTotalTime + OverheadTime',
    'PendingTime': 'Time the driver waited because the input or the precondition was not satisfied',
    'ScheduleTime': 'Driver scheduling wait time',
    'InputEmptyTime': 'Time spent waiting for an empty input',
    'FirstInputEmptyTime': 'Time spent waiting for the first empty input',
    'OutputFullTime': 'Time spent waiting for a full output',
    'PreconditionBlockTime': 'Time blocked by the precondition',
    'PipelineID': 'Pipeline identifier',
    'Depth': 'Depth of the node in the execution tree',
    'NodeType': 'Node type',
    'IsHotspot': 'Whether the node is a hotspot',
    'HotspotSeverity': 'Hotspot severity',
    'IsGroupExecution': 'Whether it is a group execution',
    'BlockByInputEmpty': 'Number of times blocked by an empty input',
    'BlockByOutputFull': 'Number of times blocked by a full output',
    'BlockByPrecondition': 'Number of times blocked by an unmet precondition',
    'DegreeOfParallelism': 'Degree of parallelism',
    'TotalDegreeOfParallelism': 'Total degree of parallelism',
    'PeakDriverQueueSize': 'Peak driver queue size',
    'ScheduleCount': 'Number of scheduling attempts',
    'YieldByLocalWait': 'Number of yields caused by local waiting',
    'YieldByPreempt': 'Number of yields caused by preemption',
    'YieldByTimeLimit': 'Number of yields caused by the time limit',
    'EnableEventScheduler': 'Whether the event scheduler is enabled',
    'InitialProcessDriverCount': 'Number of initial process drivers',
    'InitialProcessMem': 'Initial process memory',
    'JITCounter': 'JIT compilation counter',
    'JITTotalCostTime': 'Total JIT compilation time',
    'QueryMemoryLimit': 'Query memory limit',
    'BackendProfileMergeTime': 'Profile merge time on the BE',

    // ============================================
    // Operator 通用指标
    // ============================================
    'OperatorTotalTime': 'Total time consumed by the operator',
    'PushRowNum': 'Cumulative output rows of the operator',
    'PullRowNum': 'Cumulative input rows of the operator',
    'PullChunkNum': 'Cumulative input chunks of the operator',
    'PushChunkNum': 'Cumulative output chunks of the operator',
    'PullTotalTime': 'Total time of the pull operations',
    'PushTotalTime': 'Total time of the push operations',
    'PeakMemoryUsage': 'Peak memory usage of the operator',
    'OutputRows': 'Rows output by the operator',
    'TimePercentage': 'Percentage of the total time spent in this operator',
    'OutputChunkBytes': 'Bytes of the chunks output by the operator',
    'IsSubordinate': 'Whether the operator is subordinate',
    'ConjunctsInputRows': 'Input rows of the conjuncts',
    'ConjunctsOutputRows': 'Output rows of the conjuncts',
    'ConjunctsTime': 'Time spent evaluating the conjuncts',
    'RuntimeFilterNum': 'Number of runtime filters',
    'RuntimeInFilterNum': 'Number of runtime IN filters',
    'JoinRuntimeFilterEvaluate': 'Number of join runtime filter evaluations',
    'JoinRuntimeFilterHashTime': 'Time spent computing the join runtime filter hash',
    'JoinRuntimeFilterInputRows': 'Input rows of the join runtime filter',
    'JoinRuntimeFilterOutputRows': 'Output rows of the join runtime filter',
    'JoinRuntimeFilterTime': 'Time spent on the join runtime filter',
    'SetFinishingTime': 'Time spent setting the finishing state',
    'RequestReceived': 'Number of requests received',
    'SinkType': 'Sink type',
    'AppendChunkTime': 'Time spent appending chunks',
    'ResultRendTime': 'Time spent rendering the result',
    'TupleConvertTime': 'Time spent converting tuples',
    'NumSentRows': 'Number of rows sent',

    // ============================================
    // Scan Operator (OLAP/Connector)
    // ============================================
    'Table': 'Table name',
    'Rollup': 'Name of the materialized view or rollup used',
    'ScanTime': 'Cumulative scan time, scan operations run in the async I/O thread pool',
    'TabletCount': 'Number of tablets',
    'MorselsCount': 'Number of morsels',
    'PushdownPredicates': 'Number of pushdown predicates',
    'BytesRead': 'Amount of data read',
    'RowsRead': 'Number of rows read',
    'RawRowsRead': 'Number of raw rows actually scanned',
    'CompressedBytesRead': 'Amount of compressed data read',
    'UncompressedBytesRead': 'Amount of uncompressed data read',
    'IOTime': 'Cumulative I/O time',
    'IOTimeTotal': 'Total I/O time',
    'IOTimeLocalDisk': 'Time spent on I/O operations when reading from the local cache',
    'IOTimeRemote': 'Time spent on I/O operations when reading from OSS',
    'BitmapIndexFilterRows': 'Rows filtered by the bitmap index',
    'BitmapIndexFilter': 'Time spent filtering with the bitmap index',
    'BloomFilterFilterRows': 'Rows filtered by the bloom filter',
    'BloomFilterFilter': 'Time spent filtering with the bloom filter',
    'SegmentRuntimeZoneMapFilterRows': 'Rows filtered by the runtime zone map',
    'SegmentZoneMapFilterRows': 'Rows filtered by the zone map',
    'ZoneMapIndexFilterRows': 'Rows filtered by the zone map index',
    'ZoneMapIndexFiter': 'Time spent filtering with the zone map index',
    'ShortKeyFilterRows': 'Rows filtered by the short key',
    'ShortKeyFilter': 'Time spent filtering with the short key',
    'PredFilter': 'Time spent filtering by predicate',
    'PredFilterRows': 'Rows filtered by predicate',
    'DelVecFilterRows': 'Rows filtered by the deletion vector',
    'SegmentInit': 'Time spent initializing the segment',
    'SegmentRead': 'Time spent reading the segment',
    'ColumnIteratorInit': 'Time spent initializing the column iterator',
    'BlockSeek': 'Time spent seeking a block',
    'BlockSeekCount': 'Number of block seeks',
    'BlockFetch': 'Time spent fetching a block',
    'BlockFetchCount': 'Number of block fetches',
    'ChunkCopy': 'Time spent copying chunks',
    'DecompressT': 'Time spent decompressing',
    'CreateSegmentIter': 'Time spent creating the segment iterator',
    'GetDelVec': 'Time spent fetching the deletion vector',
    'GetDeltaColumnGroup': 'Time spent fetching the delta column group',
    'PrepareChunkSourceTime': 'Time spent preparing the chunk source',
    'SubmitTaskTime': 'Time spent submitting tasks',
    'SubmitTaskCount': 'Number of submitted tasks',
    'IOTaskWaitTime': 'IO task wait time',
    'IOTaskExecTime': 'IO task execution time',
    'PrefetchHitCount': 'Number of prefetch hits',
    'PrefetchPendingTime': 'Prefetch wait time',
    'PrefetchWaitFinishTime': 'Time spent waiting for the prefetch to finish',
    
    // Connector Scan (存算分离)
    'CompressedBytesReadLocalDisk': 'Amount of compressed data read from the local cache of the compute node',
    'CompressedBytesReadRemote': 'Total amount of compressed data read from OSS',
    'CompressedBytesReadRequest': 'Size of the compressed data requested',
    'CompressedBytesReadTotal': 'Total amount of compressed data read',
    'IOCountLocalDisk': 'Number of local disk IO operations',
    'IOCountRemote': 'Number of remote IO operations',
    'IOCountRequest': 'Number of requested IO operations',
    'IOCountTotal': 'Total number of IO operations',
    'PagesCountLocalDisk': 'Number of local disk pages',
    'PagesCountRemote': 'Number of remote pages',
    'PagesCountMemory': 'Number of memory pages',
    'PagesCountTotal': 'Total number of pages',
    'DataSourceType': 'Data source type',
    'SharedScan': 'Whether the scan is shared',
    'MorselQueueType': 'Morsel queue type',
    'AdaptiveIOTasks': 'Whether adaptive IO tasks are enabled',
    'PeakIOTasks': 'Peak number of IO tasks',
    'ChunkBufferCapacity': 'Chunk buffer capacity',
    'DefaultChunkBufferCapacity': 'Default chunk buffer capacity',
    'PeakChunkBufferSize': 'Peak chunk buffer size',
    'PeakChunkBufferMemoryUsage': 'Peak memory usage of the chunk buffer',
    'PeakScanTaskQueueSize': 'Peak scan task queue size',
    'RowsetsReadCount': 'Number of rowsets read',
    'SegmentsReadCount': 'Number of segments read',
    'TotalColumnsDataPageCount': 'Total number of column data pages',
    'AccessPathHits': 'Number of access path hits',
    'AccessPathUnhits': 'Number of access path misses',
    'PushdownAccessPaths': 'Number of pushdown access paths',
    'ShortKeyRangeNumber': 'Number of short key ranges',
    'RemainingRowsAfterShortKeyFilter': 'Rows remaining after the short key filter',
    'MemAllocFailed': 'Number of failed memory allocations',
    'IOStatistics': 'IO statistics',
    'ReadPKIndex': 'Time spent reading the primary key index',
    'BitmapIndexIteratorInit': 'Time spent initializing the bitmap index iterator',
    'RawInputBytes': 'Number of raw input bytes',

    // ============================================
    // Exchange Operator
    // ============================================
    // Sink
    'PartType': 'Data distribution mode: UNPARTITIONED, RANDOM, HASH_PARTITIONED or BUCKET_SHUFFLE_HASH_PARTITIONED',
    'DestFragments': 'Destination fragments',
    'DestID': 'Destination ID',
    'ChannelNum': 'Number of channels',
    'BytesSent': 'Amount of data sent',
    'BytesUnsent': 'Amount of data not sent',
    'RequestSent': 'Number of requests sent',
    'RequestUnsent': 'Number of requests not sent',
    'BytesPassThrough': 'Number of pass-through bytes',
    'PassThroughBufferPeakMemoryUsage': 'Peak memory usage of the pass-through buffer',
    'OverallThroughput': 'Throughput',
    'OverallTime': 'Overall time',
    'NetworkTime': 'Data packet transfer time (excluding post-receive processing)',
    'WaitTime': 'Time spent waiting because the sender queue was full',
    'NetworkBandwidth': 'Network bandwidth',
    'SerializeChunkTime': 'Time spent serializing chunks',
    'SerializedBytes': 'Number of serialized bytes',
    'CompressTime': 'Time spent compressing',
    'CompressedBytes': 'Number of compressed bytes',
    'RpcCount': 'Number of RPC calls',
    'RpcAvgTime': 'Average RPC time',
    'ShuffleHashTime': 'Time spent computing the shuffle hash',
    'ShuffleChunkAppendTime': 'Time spent appending shuffle chunks',
    'ShuffleChunkAppendCounter': 'Number of shuffle chunk appends',
    'BufferUnplugCount': 'Number of buffer unplugs',
    'ClosureBlockCount': 'Number of closure blocks',
    'ClosureBlockTime': 'Time blocked by the closure',
    
    // Source
    'SenderWaitLockTime': 'Time waiting for the lock',
    'WaitLockTime': 'Time waiting for the lock',
    'BytesReceived': 'Amount of data received',
    'DecompressChunkTime': 'Decompression time',
    'DeserializeChunkTime': 'Deserialization time',
    'SenderTotalTime': 'Total sending time',
    'ReceiverProcessTotalTime': 'Total processing time on the receiver side',
    'PeakBufferMemoryBytes': 'Peak buffer memory in bytes',

    // ============================================
    // Aggregate Operator
    // ============================================
    'GroupingKeys': 'GROUP BY columns',
    'AggregateFunctions': 'Aggregate functions',
    'AggComputeTime': 'Time spent computing the aggregate functions',
    'ExprComputeTime': 'Time spent computing expressions',
    'HashTableSize': 'Hash table size',
    'HashTableMemoryUsage': 'Hash table memory usage',
    'InputRowCount': 'Number of input rows',
    'PassThroughRowCount': 'Number of pass-through rows',
    'ResultAggAppendTime': 'Time spent appending the aggregated result',
    'ResultGroupByAppendTime': 'Time spent appending the grouped result',
    'GetResultsTime': 'Time spent fetching the results',
    'IteratorMergeTime': 'Time spent merging the iterators',
    'StreamingTime': 'Streaming processing time',
    'AggStateName': 'Name of the aggregate state',

    // ============================================
    // Join Operator
    // ============================================
    // Probe
    'DistributionMode': 'Data distribution mode',
    'JoinType': 'Join type',
    'OtherJoinConjunctEvaluateTime': 'Time spent evaluating other join conjuncts',
    'ProbeConjunctEvaluateTime': 'Time spent evaluating the probe conjuncts',
    'SearchHashTableTime': 'Time spent searching the hash table',
    'SearchHashTableCount': 'Number of hash table searches',
    'WhereConjunctEvaluateTime': 'Time spent evaluating the where conjuncts',
    'OutputBuildColumnTime': 'Time spent outputting the build columns',
    'OutputProbeColumnTime': 'Time spent outputting the probe columns',
    'OutputTupleColumnTime': 'Time spent outputting the tuple columns',
    'ProbeRowsCounter': 'Probe row counter',
    
    // Build
    'JoinPredicates': 'Join predicates',
    'BuildBuckets': 'Number of hash table buckets',
    'BuildHashTableTime': 'Time spent building the hash table',
    'CopyRightTableChunkTime': 'Time spent copying chunks of the right table',
    'RuntimeFilterBuildTime': 'Time spent building the runtime filter',
    'BuildConjunctEvaluateTime': 'Time spent evaluating the build conjuncts',
    'BuildRowsCounter': 'Build row counter',
    'RuntimeBloomFilterNum': 'Number of runtime bloom filters',

    // ============================================
    // Window Function Operator
    // ============================================
    'ComputeTime': 'Time spent computing the window function',
    'PartitionKeys': 'Partition columns',
    'PartitionChunksNum': 'Number of partition chunks',
    'PartitionRowsNum': 'Number of partition rows',
    'PeerGroupChunksNum': 'Number of peer group chunks',
    'PeerGroupRowsNum': 'Number of peer group rows',
    'ColumnResize': 'Number of column resizes',

    // ============================================
    // Sort Operator
    // ============================================
    'SortKeys': 'Sort keys',
    'SortType': 'How the query result is sorted: full sort or top N sort',
    'MaxBufferedRows': 'Peak number of buffered rows',
    'MaxBufferedBytes': 'Peak number of buffered bytes',
    'NumSortedRuns': 'Number of sorted runs',
    'BuildingTime': 'Time spent maintaining internal data structures during the sort',
    'MergingTime': 'Time spent merging sorted runs during the sort',
    'SortingTime': 'Time spent sorting',
    'OutputTime': 'Time spent building the sorted output sequence',

    // ============================================
    // Merge Operator
    // ============================================
    'Limit': 'Maximum number of rows returned',
    'Offset': 'Offset',
    'StreamingBatchSize': 'Amount of data processed by each merge operation when merging in streaming mode',
    'LateMaterializationMaxBufferChunkNum': 'Maximum number of chunks in the buffer when late materialization is enabled',
    'OverallStageCount': 'Total number of executions of all stages',
    'OverallStageTime': 'Total execution time of each stage',
    'LateMaterializationGenerateOrdinalTime': 'Time spent generating the ordinal column during late materialization',
    'SortedRunProviderTime': 'Time spent fetching data from the provider in the Process stage',

    // ============================================
    // TableFunction Operator
    // ============================================
    'TableFunctionExecTime': 'Time spent computing the table function',
    'TableFunctionExecCount': 'Number of table function executions',

    // ============================================
    // Project Operator
    // ============================================
    'CommonSubExprComputeTime': 'Time spent computing common subexpressions',

    // ============================================
    // LocalExchange Operator
    // ============================================
    'Type': 'Local exchange type: Passthrough, Partition or Broadcast',
    'ShuffleNum': 'Number of shuffles, only valid when Type is Partition',
    'LocalExchangePeakMemoryUsage': 'Peak memory usage',
    'LocalExchangePeakBufferSize': 'Peak buffer size',
    'LocalExchangePeakBufferMemoryUsage': 'Peak memory usage of the buffer',
    'LocalExchangePeakBufferChunkNum': 'Peak number of chunks in the buffer',
    'LocalExchangePeakBufferRowNum': 'Peak number of rows in the buffer',
    'LocalExchangePeakBufferBytes': 'Peak amount of data in the buffer',
    'LocalExchangePeakBufferChunkSize': 'Peak chunk size in the buffer',
    'LocalExchangePeakBufferChunkRowNum': 'Peak number of rows per chunk in the buffer',
    'LocalExchangePeakBufferChunkBytes': 'Peak amount of data per chunk in the buffer',

    // ============================================
    // OlapTableSink Operator
    // ============================================
    'IndexNum': 'Number of synchronous materialized views created for the target table',
    'ReplicatedStorage': 'Whether single-leader replication is enabled',
    'TxnID': 'ID of the load transaction',
    'RowsFiltered': 'Number of rows filtered out because of poor data quality',
    'RowsReturned': 'Number of rows written to the target table',
    'RpcClientSideTime': 'Total RPC time of the load, as recorded by the client',
    'RpcServerSideTime': 'Total RPC time of the load, as recorded by the server',
    'PrepareDataTime': 'Total time of the data preparation phase, including format conversion and data quality checks',
    'SendDataTime': 'Local time spent sending data, including serializing and compressing it and submitting the task to the sender queue',

    // ============================================
    // 执行时间分类 (Execution Time Categories)
    // ============================================
    'IO': 'Sum of the IO time of all SCAN nodes',
    'Processing': 'Total time operators spent on their computation',
    'IoSeekTime': 'Total time spent on IO seek addressing, only applicable to disaggregated storage-compute instances',
    'LocalDiskReadIOTime': 'I/O time spent reading data from the local cache, only applicable to disaggregated storage-compute instances',
    'RemoteReadIOTime': 'I/O time spent reading data from remote OSS, only applicable to disaggregated storage-compute instances',

  };

  // Get metric description (case-insensitive matching)
  getMetricDescription(key: string): string | null {
    // Direct match first
    if (this.metricDescriptions[key]) {
      return this.metricDescriptions[key];
    }
    // Case-insensitive match
    const lowerKey = key.toLowerCase();
    for (const [k, v] of Object.entries(this.metricDescriptions)) {
      if (k.toLowerCase() === lowerKey) {
        return v;
      }
    }
    return null;
  }

  // Profile management settings
  profileSettings = {
    mode: 'external',
    hideSubHeader: false, // Enable search
    noDataMessage: 'No profile records',
    actions: {
      add: false,
      edit: true,
      delete: false,
      position: 'right',
      width: '80px',
    },
    edit: {
      editButtonContent: '<i class="nb-search"></i>',
    },
    pager: {
      display: true,
      perPage: 20,
    },
    columns: {
      QueryId: { title: 'Query ID', type: 'string', width: '25%' },
      StartTime: { title: 'Start Time', type: 'string', width: '15%' },
      Time: {
        title: 'Duration',
        type: 'html',
        sanitizer: { bypassHtml: true },
        width: '10%',
        valuePrepareFunction: (value: string | number) => {
          // Parse StarRocks duration string to milliseconds for accurate threshold comparison
          const durationMs = parseStarRocksDuration(value);
          return renderMetricBadge(durationMs, this.profileDurationThresholds, {
            labelFormatter: (val) => {
              // Use original string for display, but parsed number for thresholds
              return typeof value === 'string' ? value : `${val}ms`;
            }
          });
        },
      },
      State: {
        title: 'Status',
        type: 'html',
        sanitizer: { bypassHtml: true },
        width: '10%',
        valuePrepareFunction: (value: string) => {
          const status = value === 'Finished' ? 'success' : 'warning';
          return `<span class="badge badge-${status}">${value}</span>`;
        },
      },
      Statement: { 
        title: 'SQL', 
        type: 'html', 
        sanitizer: { bypassHtml: true },
        width: '40%',
        valuePrepareFunction: (value: any) => renderLongText(value, 100),
      },
    },
  };

  constructor(
    private route: ActivatedRoute,
    private nodeService: NodeService,
    private clusterContextService: ClusterContextService,
    private toastrService: NbToastrService,
    private dialogService: NbDialogService,
    private authService: AuthService,
    private cdr: ChangeDetectorRef,
    private location: Location, // Inject Location
  ) {
    // Try to get clusterId from route first (for direct navigation)
    const routeClusterId = parseInt(this.route.snapshot.paramMap.get('clusterId') || '0', 10);
    this.clusterId = routeClusterId;
  }

  ngOnInit(): void {
    // Subscribe to active cluster changes
    this.clusterContextService.activeCluster$
      .pipe(takeUntil(this.destroy$))
      .subscribe(cluster => {
        this.activeCluster = cluster;
        if (cluster) {
          // Always use the active cluster (override route parameter)
          const newClusterId = cluster.id;
          if (this.clusterId !== newClusterId) {
            this.clusterId = newClusterId;
            this.loadProfiles();
          }
        }
        // Backend will handle "no active cluster" case
      });

    // Load data - backend will get active cluster automatically
    this.loadProfiles();
  }

  ngOnDestroy(): void {
    this.stopAutoRefresh();
    this.destroy$.next();
    this.destroy$.complete();
  }

  // Grafana-style: selecting an interval automatically enables auto-refresh
  // Selecting 'off' disables auto-refresh
  onRefreshIntervalChange(interval: number | 'off'): void {
    this.selectedRefreshInterval = interval;
    
    if (interval === 'off') {
      // Disable auto-refresh
      this.autoRefresh = false;
      this.stopAutoRefresh();
    } else {
      // Enable auto-refresh with selected interval
      this.autoRefresh = true;
      this.stopAutoRefresh();
      this.startAutoRefresh();
    }
  }

  startAutoRefresh(): void {
    this.stopAutoRefresh(); // Clear any existing interval
    
    // Only start if interval is a number (not 'off')
    if (typeof this.selectedRefreshInterval !== 'number') {
      return;
    }
    
    this.refreshInterval = setInterval(() => {
      // Stop auto-refresh if user is not authenticated (logged out)
      if (!this.authService.isAuthenticated()) {
        this.autoRefresh = false;
        this.selectedRefreshInterval = 'off';
        this.stopAutoRefresh();
        return;
      }
      // Only update data, don't show loading spinner during auto-refresh
      this.loadProfilesSilently();
    }, this.selectedRefreshInterval * 1000);
  }

  stopAutoRefresh(): void {
    if (this.refreshInterval) {
      clearInterval(this.refreshInterval);
      this.refreshInterval = null;
    }
  }

  // Load profiles
  loadProfiles(): void {
    this.loading = true;
    this.nodeService.listProfiles().subscribe(
      data => {
        this.profileSource.load(data);
        this.updateDynamicThresholds(data);
        this.loading = false;
      },
      error => {
        this.toastrService.danger(ErrorHandler.handleClusterError(error), 'Failed to load');
        this.loading = false;
      }
    );
  }

  // Load profiles silently (for auto-refresh, without loading spinner)
  loadProfilesSilently(): void {
    this.nodeService.listProfiles().subscribe(
      data => {
        this.profileSource.load(data);
        this.updateDynamicThresholds(data);
      },
      error => {
        // Silently handle errors during auto-refresh
        console.error('Failed to refresh profiles:', error);
      }
    );
  }

  /**
   * Update dynamic thresholds based on maximum time in current data
   * Algorithm:
   * - Find the maximum execution time in the dataset
   * - Red (danger): > max_time * 70%
   * - Yellow (warning): > max_time * 40% and <= max_time * 70%
   * - Green (success): <= max_time * 40%
   * 
   * This ensures color coding adapts to the actual data range
   */
  updateDynamicThresholds(profiles: any[]): void {
    if (!profiles || profiles.length === 0) {
      return;
    }

    // Extract duration values from profiles
    const durationValues = profiles
      .map(profile => parseStarRocksDuration(profile.Time))
      .filter(value => !isNaN(value) && value > 0);

    if (durationValues.length === 0) {
      // No valid data, use defaults
      return;
    }

    // Find maximum time
    const maxTime = Math.max(...durationValues);
    
    // Calculate thresholds based on max time percentage
    // Red: > 70% of max time
    // Yellow: > 40% of max time
    const warnThreshold = maxTime * 0.5;   // 40% of max
    const dangerThreshold = maxTime * 0.8; // 70% of max

    // Update thresholds
    this.profileDurationThresholds = {
      warn: warnThreshold,
      danger: dangerThreshold,
    };
  }

  // Helper: Format milliseconds to readable duration
  private formatDuration(ms: number): string {
    if (ms < 1000) return `${ms}ms`;
    if (ms < 60000) return `${(ms / 1000).toFixed(2)}s`;
    return `${(ms / 60000).toFixed(2)}m`;
  }

  // Handle profile edit action (view profile)
  onProfileEdit(event: any): void {
    this.viewProfileDetail(event.data.QueryId);
  }

  // View profile detail from profile list
  viewProfileDetail(queryId: string): void {
    this.currentQueryId = queryId;
    this.profileDetailLoading = true;
    this.analysisLoading = true;
    this.currentProfileDetail = '';
    this.analysisData = null;
    this.analysisError = '';
    this.topNodes = [];
    this.graphNodes = [];
    this.graphEdges = [];
    this.selectedNode = null;
    
    // Open dialog first with loading state
    this.dialogService.open(this.profileDetailDialogTemplate, {
      context: { profile: this.currentProfileDetail },
      hasBackdrop: true,
      closeOnBackdropClick: true,
      closeOnEsc: true,
      dialogClass: 'profile-dialog-lg',
    });
    
    // Load analysis data (includes profile_content)
    this.loadAnalysis(queryId);
  }
  
  // Load profile analysis for DAG (includes profile_content)
  loadAnalysis(queryId: string): void {
    this.analysisLoading = true;
    this.profileDetailLoading = true;
    this.analysisError = '';
    
    this.nodeService.analyzeProfile(queryId).subscribe({
      next: (data) => {
        this.analysisData = data;
        this.topNodes = data.summary?.top_time_consuming_nodes || [];
        // Set profile content from analysis response
        if (data.profile_content) {
          this.currentProfileDetail = data.profile_content;
        }
        if (data.execution_tree) {
          this.buildGraph(data.execution_tree);
        }
        this.analysisLoading = false;
        this.profileDetailLoading = false;
        
        // If LLM is available but pending, load it async
        if (data.llm_analysis?.available && data.llm_analysis?.status === 'pending') {
          this.loadLLMAnalysis(queryId);
        }
      },
      error: (err) => {
        console.error('Failed to analyze profile', err);
        this.analysisError = 'Analysis failed: ' + (err.error?.message || err.message || 'Unknown error');
        this.analysisLoading = false;
        this.profileDetailLoading = false;
      }
    });
  }
  
  /**
   * Load LLM analysis async (called after DAG is rendered)
   * Passes pre-analyzed data to avoid redundant profile parsing
   */
  private loadLLMAnalysis(queryId: string, forceRefresh: boolean = false): void {
    if (!this.clusterId || !this.analysisData) return;
    
    this.llmAnalysisLoading = true;
    this.llmAnalysisError = '';
    this.llmAnalysisStartTime = Date.now();
    this.llmAnalysisElapsedTime = 0;
    
    // Pass pre-analyzed data to backend to avoid re-fetching profile
    // Build payload with analysis_data and force_refresh flag
    const payload = {
      analysis_data: this.analysisData,
      force_refresh: forceRefresh || false,
    };
    
    this.nodeService.enhanceProfileWithLLM(this.clusterId, queryId, payload).subscribe({
      next: (llmData) => {
        // Calculate elapsed time
        this.llmAnalysisElapsedTime = Date.now() - this.llmAnalysisStartTime;
        
        // Clear previous error
        this.llmAnalysisError = null;
        
        // Merge LLM analysis into existing data
        if (this.analysisData) {
          this.analysisData.llm_analysis = llmData;
          // Store elapsed time in analysis data
          if (this.analysisData.llm_analysis) {
            this.analysisData.llm_analysis.elapsed_time_ms = this.llmAnalysisElapsedTime;
          }
        }
        this.llmAnalysisLoading = false;
        this.cdr.detectChanges();
      },
      error: (err) => {
        console.error('Failed to load LLM analysis', err);
        this.llmAnalysisElapsedTime = Date.now() - this.llmAnalysisStartTime;
        this.llmAnalysisError = err.error?.message || 'Failed to load LLM analysis';
        this.llmAnalysisLoading = false;
        // Update llm_analysis status
        if (this.analysisData?.llm_analysis) {
          this.analysisData.llm_analysis.status = 'failed';
          this.analysisData.llm_analysis.available = false;
        }
        this.cdr.detectChanges();
      }
    });
  }
  
  /**
   * Retry LLM analysis (force refresh)
   */
  retryLLMAnalysis(): void {
    if (this.currentQueryId) {
      this.loadLLMAnalysis(this.currentQueryId, true);
    }
  }
  
  /**
   * Get LLM analysis elapsed time in formatted string
   */
  getLLMElapsedTime(): string {
    const llmData = this.analysisData?.llm_analysis;
    const elapsedMs = llmData?.elapsed_time_ms || this.llmAnalysisElapsedTime;
    
    if (!elapsedMs) return '';
    
    if (elapsedMs < 1000) {
      return `${elapsedMs}ms`;
    } else {
      return `${(elapsedMs / 1000).toFixed(1)}s`;
    }
  }
  
  /**
   * Check if LLM analysis was cached
   */
  isLLMAnalysisCached(): boolean {
    return this.analysisData?.llm_analysis?.from_cache === true;
  }
  
  // Refresh analysis
  refreshAnalysis(): void {
    if (this.currentQueryId) {
      this.loadAnalysis(this.currentQueryId);
    }
  }

  private getIntersectionPoint(
    from: { x: number, y: number },
    target: { x: number, y: number, width: number, height: number }
  ): { x: number, y: number } {
    const dx = from.x - target.x;
    const dy = from.y - target.y;
    if (dx === 0 && dy === 0) {
      return { ...target };
    }

    const halfW = target.width / 2;
    const halfH = target.height / 2;
    const tx = dx !== 0 ? halfW / Math.abs(dx) : Infinity;
    const ty = dy !== 0 ? halfH / Math.abs(dy) : Infinity;
    const t = Math.min(tx, ty);

    return {
      x: target.x + t * dx,
      y: target.y + t * dy,
    };
  }

  // Build DAG graph using dagre
  buildGraph(tree: any): void {
    const g = new dagre.graphlib.Graph();
    g.setGraph({ 
      rankdir: this.graphDirection,
      marginx: 40, 
      marginy: 40,
      nodesep: 80,  // Increase node separation
      ranksep: 100  // Increase rank separation
    });
    g.setDefaultEdgeLabel(() => ({}));

    // Create a shallow copy of nodes array to avoid mutating the original data
    const nodeList = [...(tree.nodes || [])];
    if (tree.root) {
      // Check if root is already in the list to avoid duplicates
      if (!nodeList.find((n: any) => n.id === tree.root.id)) {
        nodeList.unshift(tree.root);
      }
    }

    console.log('Building graph with nodes:', nodeList.length);

    // Add nodes
    nodeList.forEach((node: any) => {
      // Node height: Header(35) + Body(55) + Progress(3) ≈ 93px
      // Use 95px for dagre layout to ensure proper spacing
      g.setNode(node.id, { width: 220, height: 95 });
    });

    // Add edges
    nodeList.forEach((node: any) => {
      if (node.children) {
        node.children.forEach((childId: string) => {
          g.setEdge(childId, node.id);
        });
      }
    });

    // Calculate layout
    dagre.layout(g);

    console.log('Graph layout complete:', {
      width: g.graph().width,
      height: g.graph().height,
      nodes: g.nodes().length,
      edges: g.edges().length
    });

    // Extract coordinates - round to integer pixels to prevent subpixel blur
    this.graphNodes = nodeList.map((node: any) => {
      const layoutNode = g.node(node.id);
      
      // Calculate coordinates for sanitized output
      // Round to integers to avoid subpixel rendering blur
      const x = (layoutNode && isFinite(layoutNode.x)) ? Math.round(layoutNode.x) : 0;
      const y = (layoutNode && isFinite(layoutNode.y)) ? Math.round(layoutNode.y) : 0;
      const width = (layoutNode && isFinite(layoutNode.width)) ? Math.round(layoutNode.width) : 220;
      const height = (layoutNode && isFinite(layoutNode.height)) ? Math.round(layoutNode.height) : 95;

      return {
        ...node,
        x,
        y,
        width,
        height
      };
    });

    // First pass: collect all rows values to determine max for stroke width calculation
    const allRows = nodeList.map((n: any) => n.rows || 0).filter((r: number) => r > 0);
    const maxRows = allRows.length > 0 ? Math.max(...allRows) : 1;

    this.graphEdges = g.edges().map((e: any, index: number) => {
      const edge = g.edge(e);
      const sourceNodeData = nodeList.find((n: any) => n.id === e.v);
      const sourceNode = this.graphNodes.find((n: any) => n.id === e.v);
      const targetNode = this.graphNodes.find((n: any) => n.id === e.w);
      const rows = sourceNodeData?.rows || 0;

      let labelFormatted = rows > 0 ? 'Rows: ' + Number(rows).toLocaleString() : '';

      if (targetNode && (
          targetNode.operator_name.includes('JOIN') ||
          targetNode.operator_name === 'NESTLOOP_JOIN' ||
          targetNode.operator_name === 'HASH_JOIN'
        ) && targetNode.children && targetNode.children.length >= 2) {
        if (targetNode.children[0] === e.v) {
          labelFormatted += ' (PROBE)';
        } else if (targetNode.children[1] === e.v) {
          labelFormatted += ' (BUILD)';
        }
      }

      // Calculate dynamic stroke width based on rows (Algorithm: Logarithmic Scale)
      // Use Log scale because rows can vary from 0 to billions. Linear scale makes small diffs invisible.
      const minStrokeWidth = 2;
      const maxStrokeWidth = 9;
      
      let strokeWidth = minStrokeWidth;
      if (rows > 0 && maxRows > 0) {
        const logRows = Math.log10(rows + 1); // +1 to avoid log(0)
        const logMax = Math.log10(maxRows + 1);
        // Calculate ratio based on orders of magnitude
        const ratio = logMax > 0 ? logRows / logMax : 0;
        strokeWidth = minStrokeWidth + ratio * (maxStrokeWidth - minStrokeWidth);
      }

      // Calculate target position for multi-child nodes (Algorithm: Center Distribution)
      // Instead of spreading across the full width, distribute from center with fixed spacing
      // This looks better and more "connected"
      let targetX = targetNode?.x || 0;
      if (targetNode && targetNode.children && targetNode.children.length > 1) {
        const childIndex = targetNode.children.indexOf(e.v);
        const numChildren = targetNode.children.length;
        const spacing = 40; // 40px spacing between connection points
        
        // Calculate offset from center
        // e.g. 2 children: -20, +20
        // e.g. 3 children: -40, 0, +40
        const centerOffset = (childIndex - (numChildren - 1) / 2) * spacing;
        targetX = targetNode.x + centerOffset;
      }

      // Calculate visible line segment
      let visibleStart = { x: 0, y: 0 };
      let visibleEnd = { x: 0, y: 0 };
      
      if (sourceNode && targetNode) {
        // Source: arrow starts from node's TOP edge
        visibleStart = {
          x: sourceNode.x,
          y: sourceNode.y - sourceNode.height / 2
        };
        // Target: arrow ends at node's BOTTOM edge
        // Use dagre height/2 as base, will be fine-tuned by updateEdgesAfterRender()
        visibleEnd = {
          x: targetX,
          y: targetNode.y + targetNode.height / 2
        };
      }

      // Determine stroke color based on target node type
      let strokeColor = themeColor('--border-basic-color-4', '#c5cee0');
      if (targetNode) {
        const name = targetNode.operator_name?.toUpperCase() || '';
        if (name.includes('SCAN') || name.includes('JOIN')) {
          strokeColor = themeColor('--color-warning-default', '#ffaa00');
        }
      }

      // Calculate arrow size based on stroke width
      // Arrow must be larger than stroke width to avoid being covered
      const arrowSize = Math.max(12, strokeWidth * 2.5);
      const arrowRefY = arrowSize / 2;
      // Polygon points: triangle pointing right - (0,0) -> (arrowSize, arrowSize/2) -> (0, arrowSize)
      const arrowPoints = `0,0 ${arrowSize},${arrowRefY} 0,${arrowSize}`;
      // refX = 0: arrow BASE aligns to path end, arrow TIP extends to original target
      const arrowRefX = 0;

      // Build edge path: For multi-child parents, use vertical-then-diagonal pattern (like Figure 2)
      // Path: childTop -> verticalLift -> parentBottom (3 points: vertical segment + diagonal segment)
      const hasMultipleChildren = targetNode?.children && targetNode.children.length >= 2;
      let displayPoints: { x: number, y: number }[] = [visibleStart, visibleEnd];

      if (hasMultipleChildren) {
        // Calculate vertical lift distance (how far to go up before turning diagonal)
        const deltaY = Math.abs(visibleStart.y - visibleEnd.y);
        const liftDistance = Math.min(50, deltaY * 0.35); // 35% of vertical distance, max 50px
        
        // Middle point: directly above source node (vertical segment)
        const middlePoint = {
          x: visibleStart.x,
          y: visibleStart.y - liftDistance
        };
        
        // Three-point path: vertical up, then diagonal to target
        displayPoints = [visibleStart, middlePoint, visibleEnd];
      }

      // **KEY FIX**: Shorten the path by arrowSize so the stroke doesn't overlap the arrow
      // The arrow marker will "extend" from the shortened end to the original target
      if (displayPoints.length >= 2) {
        const lastIdx = displayPoints.length - 1;
        const prevIdx = lastIdx - 1;
        const dx = displayPoints[lastIdx].x - displayPoints[prevIdx].x;
        const dy = displayPoints[lastIdx].y - displayPoints[prevIdx].y;
        const segmentLen = Math.sqrt(dx * dx + dy * dy);
        
        if (segmentLen > arrowSize) {
          // Shorten the last segment by arrowSize
          const shortenRatio = (segmentLen - arrowSize) / segmentLen;
          displayPoints[lastIdx] = {
            x: displayPoints[prevIdx].x + dx * shortenRatio,
            y: displayPoints[prevIdx].y + dy * shortenRatio
          };
        }
      }

      // Label position: geometric middle of the path
      const labelPos = this.getEdgeLabelPosition(displayPoints);

      return {
        v: e.v,
        w: e.w,
        points: displayPoints,
        markerId: `edge-marker-${e.v}-${e.w}-${index}`,
        labelPos,
        strokeColor,
        strokeWidth,
        arrowSize,
        arrowRefX,
        arrowRefY,
        arrowPoints,
        label: rows,
        labelFormatted,
      };
    });
    
    // Calculate bounding box
    const maxX = Math.max(...this.graphNodes.map((n: any) => (n.x || 0) + (n.width || 0)/2));
    const maxY = Math.max(...this.graphNodes.map((n: any) => (n.y || 0) + (n.height || 0)/2));
    this.graphWidth = Math.max(maxX + 50, 600);
    this.graphHeight = Math.max(maxY + 50, 400);
    
    // Force change detection to update view
    this.cdr.markForCheck();
    
    // DEBUG: Log final results
    console.log('=== Build complete ===');
    console.log('graphNodes count:', this.graphNodes.length);
    console.log('graphEdges count:', this.graphEdges.length);
    console.log('Chart size:', this.graphWidth, 'x', this.graphHeight);
    if (this.graphNodes.length > 0) {
      console.log('First node:', {
        id: this.graphNodes[0].id,
        operator_name: this.graphNodes[0].operator_name,
        x: this.graphNodes[0].x,
        y: this.graphNodes[0].y,
        width: this.graphNodes[0].width,
        height: this.graphNodes[0].height
      });
    }
    console.log('=============');
    
    // Calculate node ranks for color coding
    this.calculateNodeRanks();
    
    // Center the graph after layout, then update edges based on actual DOM heights
    setTimeout(() => {
      this.centerGraph();
      this.updateEdgesAfterRender();
    });
  }
  
  // Update edge positions after DOM has rendered (measure actual node heights)
  private updateEdgesAfterRender(): void {
    const nodeElements = document.querySelectorAll('.dag-node');
    if (!nodeElements.length) return;
    
    // Build a map of actual DOM heights
    // getBoundingClientRect returns scaled size, so divide by zoomLevel to get original
    const actualHeights: Map<string, number> = new Map();
    nodeElements.forEach((el: Element) => {
      const nodeId = el.getAttribute('data-node-id');
      if (nodeId) {
        actualHeights.set(nodeId, el.getBoundingClientRect().height / this.zoomLevel);
      }
    });
    
    // Update graphNodes with actual heights
    this.graphNodes.forEach(node => {
      const actualH = actualHeights.get(node.id);
      if (actualH && actualH > 0) {
        node.actualHeight = actualH;
      }
    });
    
    // Recalculate edge endpoints using actual heights
    this.graphEdges = this.graphEdges.map(edge => {
      const sourceNode = this.graphNodes.find(n => n.id === edge.v);
      const targetNode = this.graphNodes.find(n => n.id === edge.w);
      
      if (sourceNode && targetNode && edge.points && edge.points.length >= 2) {
        const targetActualH = targetNode.actualHeight || targetNode.height;
        
        // Source node: arrow starts from its TOP edge
        const newStartY = sourceNode.y - sourceNode.height / 2;
        // Target node: arrow ends at its BOTTOM edge (original target position)
        const originalEndY = (targetNode.y - targetNode.height / 2) + targetActualH;
        
        const hasMultipleChildren = targetNode.children && targetNode.children.length >= 2;
        
        if (hasMultipleChildren && edge.points.length === 3) {
          // 3-point path: [start, middle, end] - vertical + diagonal pattern
          const deltaY = Math.abs(newStartY - originalEndY);
          const liftDistance = Math.min(50, deltaY * 0.35);
          
          edge.points[0] = { x: edge.points[0].x, y: newStartY };
          edge.points[1] = { x: edge.points[0].x, y: newStartY - liftDistance };
          edge.points[2] = { x: edge.points[2].x, y: originalEndY };
        } else {
          // 2-point path: direct line
          edge.points[0] = { x: edge.points[0].x, y: newStartY };
          edge.points[edge.points.length - 1] = { x: edge.points[edge.points.length - 1].x, y: originalEndY };
        }
        
        // **KEY FIX**: Shorten the last segment by arrowSize so stroke doesn't overlap arrow
        const arrowSize = edge.arrowSize || 12;
        const lastIdx = edge.points.length - 1;
        const prevIdx = lastIdx - 1;
        const dx = edge.points[lastIdx].x - edge.points[prevIdx].x;
        const dy = edge.points[lastIdx].y - edge.points[prevIdx].y;
        const segmentLen = Math.sqrt(dx * dx + dy * dy);
        
        if (segmentLen > arrowSize) {
          const shortenRatio = (segmentLen - arrowSize) / segmentLen;
          edge.points[lastIdx] = {
            x: edge.points[prevIdx].x + dx * shortenRatio,
            y: edge.points[prevIdx].y + dy * shortenRatio
          };
        }
        
        // Update label position
        edge.labelPos = this.getEdgeLabelPosition(edge.points);
      }
      
      return edge;
    });
    console.log('Edges updated with actual DOM heights');
    this.cdr.markForCheck();
  }
  
  // Center the graph in the viewport
  centerGraph(): void {
    const viewport = document.querySelector('.dag-center-panel') as HTMLElement;
    if (viewport && this.graphWidth > 0 && this.graphHeight > 0) {
      const vw = viewport.clientWidth;
      const vh = viewport.clientHeight;
      // Calculate scaled dimensions
      const scaledWidth = this.graphWidth * this.zoomLevel;
      const scaledHeight = this.graphHeight * this.zoomLevel;
      // Center the graph
      this.translateX = Math.round((vw - scaledWidth) / 2);
      this.translateY = Math.round((vh - scaledHeight) / 2);
      // Ensure some padding if content is larger than viewport
      if (this.translateY < 20) this.translateY = 20;
      if (this.translateX < 20) this.translateX = 20;
    }
  }
  
  // Select a node
  selectNode(node: any, event?: Event): void {
    if (event) {
      event.stopPropagation(); // Prevent panel click from clearing selection
    }
    this.selectedNode = node;
  }
  
  // Handle click on DAG panel background (clear selection)
  onDagPanelClick(event: Event): void {
    // Only clear if clicking on the panel itself, not on nodes
    const target = event.target as HTMLElement;
    if (!target.closest('.dag-node') && !target.closest('.dag-toolbar')) {
      this.clearSelectedNode();
    }
  }
  
  // Select node by ID (for top10 table click)
  selectNodeById(nodeId: string): void {
    const node = this.graphNodes.find(n => n.id === nodeId);
    if (node) {
      this.selectedNode = node;
    }
  }
  
  // Clear selected node (return to summary view)
  clearSelectedNode(): void {
    this.selectedNode = null;
  }
  
  // Get top 10 nodes by time percentage (include all nodes, even 0%)
  get top10NodesByTime(): any[] {
    if (!this.graphNodes || this.graphNodes.length === 0) return [];
    return [...this.graphNodes]
      .filter(n => n.time_percentage !== undefined && n.time_percentage !== null)
      .sort((a, b) => (b.time_percentage || 0) - (a.time_percentage || 0))
      .slice(0, 10);
  }
  
  // Get top 10 nodes by memory usage (show all nodes, even with null/0 memory)
  get top10NodesByMemory(): any[] {
    if (!this.graphNodes || this.graphNodes.length === 0) return [];
    return [...this.graphNodes]
      .sort((a, b) => (b.metrics?.memory_usage || 0) - (a.metrics?.memory_usage || 0))
      .slice(0, 10);
  }
  
  // Calculate scan time percentage
  getScanTimePercentage(): number {
    const scanMs = this.analysisData?.summary?.query_cumulative_scan_time_ms || 0;
    const totalMs = this.analysisData?.summary?.query_cumulative_operator_time_ms || 
                    this.analysisData?.summary?.query_execution_wall_time_ms || 1;
    return totalMs > 0 ? Math.min((scanMs / totalMs) * 100, 100) : 0;
  }
  
  // Calculate CPU time percentage
  getCpuTimePercentage(): number {
    const cpuMs = this.analysisData?.summary?.query_cumulative_cpu_time_ms || 0;
    const totalMs = this.analysisData?.summary?.query_cumulative_operator_time_ms || 
                    this.analysisData?.summary?.query_execution_wall_time_ms || 1;
    return totalMs > 0 ? Math.min((cpuMs / totalMs) * 100, 100) : 0;
  }

  // Calculate schedule time percentage
  getScheduleTimePercentage(): number {
    const scheduleMs = this.analysisData?.summary?.query_peak_schedule_time_ms || 0;
    const totalMs = this.analysisData?.summary?.query_execution_wall_time_ms || 1;
    return totalMs > 0 ? Math.min((scheduleMs / totalMs) * 100, 100) : 0;
  }

  // Calculate network time percentage
  getNetworkTimePercentage(): number {
    const networkMs = this.analysisData?.summary?.query_cumulative_network_time_ms || 0;
    const totalMs = this.analysisData?.summary?.query_execution_wall_time_ms || 1;
    return totalMs > 0 ? Math.min((networkMs / totalMs) * 100, 100) : 0;
  }

  // Calculate result deliver time percentage
  getResultDeliverTimePercentage(): number {
    const deliverMs = this.analysisData?.summary?.result_deliver_time_ms || 0;
    const totalMs = this.analysisData?.summary?.query_execution_wall_time_ms || 1;
    return totalMs > 0 ? Math.min((deliverMs / totalMs) * 100, 100) : 0;
  }
  
  // Format time string with percentage
  formatTimeWithPercent(timeStr: string | undefined, percent: number): string {
    if (!timeStr) return '-';
    return `${percent.toFixed(1)}%(${timeStr})`;
  }

  // Check if IO metrics are available (for disaggregated storage)
  hasIoMetrics(): boolean {
    const s = this.analysisData?.summary;
    return !!(s?.io_seek_time || s?.local_disk_read_io_time || s?.remote_read_io_time);
  }

  // Check if IO statistics are available
  hasIoStatistics(): boolean {
    const s = this.analysisData?.summary;
    return !!(s?.total_raw_rows_read || s?.total_bytes_read || 
              s?.pages_count_memory || s?.pages_count_local_disk || s?.pages_count_remote ||
              s?.result_rows || s?.result_bytes);
  }

  // Scroll to diagnosis section
  scrollToDiagnosis(): void {
    this.isDiagnosisCollapsed = false;
    setTimeout(() => {
      const diagSection = document.querySelector('.diagnosis-section');
      if (diagSection) {
        diagSection.scrollIntoView({ behavior: 'smooth', block: 'start' });
      }
    }, 100);
  }
  
  // Check if any summary metric is available
  hasAnyMetric(): boolean {
    const s = this.analysisData?.summary;
    if (!s) return false;
    return !!(s.query_execution_wall_time || s.query_cumulative_operator_time ||
              s.query_cumulative_cpu_time || s.query_cumulative_scan_time ||
              s.query_cumulative_network_time || s.query_peak_schedule_time ||
              s.result_deliver_time || s.query_sum_memory_usage || s.query_spill_bytes ||
              s.datacache_hit_rate != null);
  }
  
  // Get DataCache tooltip with detailed metrics
  getDataCacheTooltip(): string {
    const s = this.analysisData?.summary;
    if (!s) return '';
    
    const local = s.datacache_bytes_local_display || '0 B';
    const remote = s.datacache_bytes_remote_display || '0 B';
    const hitRate = s.datacache_hit_rate != null ? (s.datacache_hit_rate * 100).toFixed(1) : '0';
    
    return `DataCache hit rate on a disaggregated storage-compute cluster\n` +
           `Local cache reads: ${local}\n` +
           `Remote storage reads: ${remote}\n` +
           `Hit rate: ${hitRate}%\n` +
           `(hit rate >= 70% is healthy)`;
  }
  
  // Get metric keys from metrics object (filter out null/undefined and specialized)
  getMetricKeys(metrics: any): string[] {
    if (!metrics) return [];
    return Object.keys(metrics).filter(key => {
      const val = metrics[key];
      return val !== null && val !== undefined && key !== 'specialized';
    });
  }
  
  // Format metric value based on key name
  formatMetricValue(key: string, value: any): string {
    if (value === null || value === undefined) return '-';
    const lowerKey = key.toLowerCase();
    // Time metrics (in nanoseconds)
    if (lowerKey.includes('time')) {
      return this.formatDurationNs(value);
    }
    // Memory/bytes metrics
    if (lowerKey.includes('memory') || lowerKey.includes('bytes')) {
      return this.formatBytes(value);
    }
    // Number metrics
    if (typeof value === 'number') {
      return value.toLocaleString();
    }
    return String(value);
  }

  // ============================================
  // Node Detail View Methods
  // ============================================

  // Get operator type from node name
  getOperatorType(node: any): string {
    if (!node?.operator_name) return 'DEFAULT';
    const name = node.operator_name.toUpperCase();
    if (name.includes('SCAN')) return 'SCAN';
    if (name.includes('JOIN')) return 'JOIN';
    if (name.includes('AGG')) return 'AGGREGATE';
    if (name.includes('EXCHANGE')) return 'EXCHANGE';
    if (name.includes('SORT') || name.includes('TOP')) return 'SORT';
    if (name.includes('PROJECT')) return 'PROJECT';
    return 'DEFAULT';
  }

  // Get execution time breakdown - dynamically detect from node's own metrics
  // If node has ScanTime -> show CPUTime + ScanTime
  // If node has NetworkTime -> show CPUTime + NetworkTime
  // Otherwise -> show CPUTime only (100%)
  getExecutionTimeBreakdown(): { items: { label: string; time: string; percent: number; color: string }[] } | null {
    if (!this.selectedNode) return null;
    
    const allMetrics = this.getAllNodeMetrics();
    const items: { label: string; time: string; percent: number; color: string }[] = [];
    
    // Get CPUTime from OperatorTotalTime
    const cpuTimeStr = allMetrics['__MAX_OF_OperatorTotalTime'] || allMetrics['OperatorTotalTime'] || '0ns';
    const cpuTimeNs = this.parseTimeToNs(cpuTimeStr);
    
    // Dynamically detect secondary time from the node's own metrics
    let secondaryTimeStr = '';
    let secondaryLabel = '';
    let secondaryTimeNs = 0;
    
    // Check what time metrics this node actually has
    if (allMetrics['ScanTime'] && this.parseTimeToNs(allMetrics['ScanTime']) > 0) {
      secondaryTimeStr = allMetrics['ScanTime'];
      secondaryLabel = 'ScanTime';
      secondaryTimeNs = this.parseTimeToNs(secondaryTimeStr);
    } else if (allMetrics['NetworkTime'] && this.parseTimeToNs(allMetrics['NetworkTime']) > 0) {
      secondaryTimeStr = allMetrics['NetworkTime'];
      secondaryLabel = 'NetworkTime';
      secondaryTimeNs = this.parseTimeToNs(secondaryTimeStr);
    }
    
    const totalTimeNs = cpuTimeNs + secondaryTimeNs || 1;
    
    // Add CPUTime
    items.push({
      label: 'CPUTime',
      time: cpuTimeStr,
      percent: secondaryTimeNs > 0 ? (cpuTimeNs / totalTimeNs) * 100 : 100,
      color: 'cpu'
    });
    
    // Add secondary time if this node has it
    if (secondaryLabel && secondaryTimeNs > 0) {
      items.push({
        label: secondaryLabel,
        time: secondaryTimeStr,
        percent: (secondaryTimeNs / totalTimeNs) * 100,
        color: 'secondary'
      });
    }
    
    return { items };
  }

  // Get all metrics from node (combining all sources)
  private getAllNodeMetrics(): { [key: string]: any } {
    if (!this.selectedNode) return {};
    
    // Combine metrics from node itself
    const result: { [key: string]: any } = {
      ...this.selectedNode.metrics,
      ...this.selectedNode.unique_metrics,
      ...this.selectedNode.aggregated_metrics
    };
    
    // Also get metrics from operators in pipeline
    const operators = this.getNodeOperatorsFromPipeline();
    for (const op of operators) {
      if (op.common_metrics) {
        Object.assign(result, op.common_metrics);
      }
      if (op.unique_metrics) {
        Object.assign(result, op.unique_metrics);
      }
    }
    
    return result;
  }

  // Get core metrics - directly from node's unique_metrics (no hardcoding!)
  // Filter out: time metrics (shown in Over Consuming), memory metrics (shown in Memory section), index metrics (shown in Indexes)
  getNodeCoreMetrics(): { key: string; value: any }[] {
    if (!this.selectedNode) return [];
    
    const allMetrics = this.getAllNodeMetrics();
    const result: { key: string; value: any }[] = [];
    
    // Keys to exclude (shown in other sections)
    const excludePatterns = [
      /Time$/i, /^__/, /Memory/i, /Peak/i, /FilterRows$/i,
      /^OperatorTotalTime$/, /^PullTotalTime$/, /^PushTotalTime$/
    ];
    
    // Priority keys to show first (common important metrics)
    const priorityKeys = [
      'RowsRead', 'BytesRead', 'CompressedBytesRead', 'ScanTime', 'IOTime',
      'Table', 'Rollup', 'Predicates',
      'BytesSent', 'BytesReceived', 'NetworkTime', 'WaitTime', 'OverallThroughput', 'PartType',
      'PullRowNum', 'PushRowNum', 'OutputRows'
    ];
    
    // First add priority keys that exist
    for (const key of priorityKeys) {
      const value = allMetrics[key];
      if (value !== undefined && value !== null && value !== '' && value !== '0' && value !== '0.000 B') {
        // Skip if matches exclude pattern
        if (!excludePatterns.some(p => p.test(key))) {
          result.push({ key, value });
        }
      }
    }
    
    // Then add other unique_metrics that weren't added
    const addedKeys = new Set(result.map(r => r.key));
    for (const [key, value] of Object.entries(allMetrics)) {
      if (addedKeys.has(key)) continue;
      if (value === undefined || value === null || value === '') continue;
      if (excludePatterns.some(p => p.test(key))) continue;
      
      // Only add non-zero values
      if (value !== '0' && value !== '0.000 B' && value !== 0) {
        result.push({ key, value });
      }
    }
    
    return result;
  }

  // Get memory metrics - dynamically find all memory-related metrics from node
  getNodeMemoryMetrics(): { key: string; value: any }[] {
    if (!this.selectedNode) return [];
    
    const allMetrics = this.getAllNodeMetrics();
    const result: { key: string; value: any }[] = [];
    
    // Find all metrics containing Memory or Peak (memory-related)
    for (const [key, value] of Object.entries(allMetrics)) {
      if (key.startsWith('__')) continue; // Skip aggregation keys
      if (!/Memory|Peak/i.test(key)) continue;
      if (value === undefined || value === null || value === '') continue;
      if (value === '0' || value === '0.000 B' || value === 0 || value === '0Bytes') continue;
      
      result.push({ key, value });
    }
    
    return result;
  }

  // Get index metrics - dynamically find all *FilterRows metrics from node
  getNodeIndexMetrics(): { key: string; value: any }[] {
    if (!this.selectedNode) return [];
    
    const allMetrics = this.getAllNodeMetrics();
    const result: { key: string; value: any }[] = [];
    
    // Index display name mapping
    const indexMapping: { [key: string]: string } = {
      'ShortKeyFilterRows': 'ShortKey',
      'ZoneMapIndexFilterRows': 'ZoneMap',
      'SegmentZoneMapFilterRows': 'ZoneMap',
      'SegmentRuntimeZoneMapFilterRows': 'RuntimeZoneMap',
      'BitmapIndexFilterRows': 'Bitmap',
      'BloomFilterFilterRows': 'Bloom'
    };
    
    // Find all FilterRows metrics
    for (const [sourceKey, displayKey] of Object.entries(indexMapping)) {
      const value = allMetrics[sourceKey];
      if (value !== undefined) {
        result.push({ key: displayKey, value });
      }
    }
    
    return result;
  }

  // Get over consuming metrics - dynamically find all time-related metrics from node
  // Format: "value[max=xxx, min=xxx]" like official UI
  getOverConsumingMetrics(): { key: string; value: string; indent?: number }[] {
    if (!this.selectedNode) return [];
    
    const allMetrics = this.getAllNodeMetrics();
    const result: { key: string; value: string; indent?: number }[] = [];
    
    // Time pattern: ends with Time or contains time units (ns/us/ms/s/m/h)
    const isTimeMetric = (key: string, value: any): boolean => {
      if (key.startsWith('__')) return false;
      if (!/Time$/i.test(key)) return false;
      if (key === 'OperatorTotalTime') return false; // Already shown in Execution time
      return true;
    };
    
    // Collect all time metrics from this node
    const timeMetrics: { key: string; value: string; timeNs: number }[] = [];
    
    for (const [key, value] of Object.entries(allMetrics)) {
      if (!isTimeMetric(key, value)) continue;
      if (value === undefined || value === null || value === '') continue;
      
      const timeNs = this.parseTimeToNs(value);
      if (timeNs <= 0) continue;
      
      // Build display value with max/min if available
      const maxVal = allMetrics[`__MAX_OF_${key}`];
      const minVal = allMetrics[`__MIN_OF_${key}`];
      let displayValue = String(value);
      if (maxVal && minVal) {
        displayValue = `${value}[max=${maxVal}, min=${minVal}]`;
      }
      
      timeMetrics.push({ key, value: displayValue, timeNs });
    }
    
    // Sort by time (descending) - show most time-consuming first
    timeMetrics.sort((a, b) => b.timeNs - a.timeNs);
    
    // Add to result
    for (const metric of timeMetrics) {
      result.push({ key: metric.key, value: metric.value, indent: 0 });
    }
    
    return result;
  }

  // Get all metrics for node detail view
  getNodeDetailMetrics(): { category: string; metrics: { key: string; value: any; description?: string }[] }[] {
    if (!this.selectedNode) return [];
    
    const categories: { category: string; metrics: { key: string; value: any; description?: string }[] }[] = [];
    
    // Unique metrics (non-aggregated)
    if (this.selectedNode.unique_metrics && Object.keys(this.selectedNode.unique_metrics).length > 0) {
      const metrics = Object.entries(this.selectedNode.unique_metrics)
        .filter(([_, v]) => v !== null && v !== undefined)
        .map(([key, value]) => ({
          key,
          value,
          description: this.metricDescriptions[key]
        }));
      if (metrics.length > 0) {
        categories.push({ category: 'Unique metrics', metrics });
      }
    }
    
    // Aggregated metrics (with min/max/avg)
    if (this.selectedNode.aggregated_metrics && Object.keys(this.selectedNode.aggregated_metrics).length > 0) {
      const metrics = Object.entries(this.selectedNode.aggregated_metrics)
        .filter(([_, v]) => v !== null && v !== undefined)
        .map(([key, value]) => ({
          key,
          value,
          description: this.metricDescriptions[key]
        }));
      if (metrics.length > 0) {
        categories.push({ category: 'Aggregated metrics', metrics });
      }
    }
    
    return categories;
  }

  // Get pipeline info for selected node
  getNodePipelineInfo(): any[] {
    if (!this.selectedNode) return [];
    
    // Pipeline info is typically in the node's children or specialized metrics
    const pipelineMetrics = [
      'DriverTotalTime', 'ActiveTime', 'PendingTime', 'ScheduleTime',
      'InputEmptyTime', 'OutputFullTime', 'PreconditionBlockTime',
      'LocalRfWaitingTime', 'PipelineID', 'DriverNum'
    ];
    
    const result: { key: string; value: any }[] = [];
    const allMetrics = {
      ...this.selectedNode.unique_metrics,
      ...this.selectedNode.aggregated_metrics
    };
    
    for (const key of pipelineMetrics) {
      if (allMetrics[key] !== undefined && allMetrics[key] !== null) {
        result.push({ key, value: allMetrics[key] });
      }
    }
    
    return result;
  }

  // Parse time string to nanoseconds (e.g., "84.535us" -> 84535, "1s183ms" -> 1183000000)
  private parseTimeToNs(timeStr: any): number {
    if (!timeStr) return 0;
    if (typeof timeStr === 'number') return timeStr;
    
    const str = String(timeStr).trim();
    let totalNs = 0;
    
    // Match patterns like "1h30m", "1s183ms", "84.535us", "0ns"
    const patterns = [
      { regex: /([\d.]+)h/i, multiplier: 3600000000000 },
      { regex: /([\d.]+)m(?!s)/i, multiplier: 60000000000 },
      { regex: /([\d.]+)s(?!$)/i, multiplier: 1000000000 },
      { regex: /([\d.]+)ms/i, multiplier: 1000000 },
      { regex: /([\d.]+)us/i, multiplier: 1000 },
      { regex: /([\d.]+)ns/i, multiplier: 1 },
      { regex: /^([\d.]+)s$/i, multiplier: 1000000000 }, // Just seconds
    ];
    
    for (const { regex, multiplier } of patterns) {
      const match = str.match(regex);
      if (match) {
        totalNs += parseFloat(match[1]) * multiplier;
      }
    }
    
    return totalNs;
  }

  // Calculate IO percentage for node (IO time / Total time)
  getNodeIoPercentage(): number {
    if (!this.selectedNode) return 0;
    
    // Get IOTime from various sources
    const ioTimeStr = this.selectedNode.aggregated_metrics?.IOTime || 
                      this.selectedNode.unique_metrics?.IOTime ||
                      this.selectedNode.metrics?.IOTime || 0;
    const ioTimeNs = this.parseTimeToNs(ioTimeStr);
    
    // Get total operator time
    const totalTimeNs = this.selectedNode.metrics?.operator_total_time_ns || 
                        this.parseTimeToNs(this.selectedNode.metrics?.operator_total_time) || 1;
    
    return totalTimeNs > 0 ? Math.min((ioTimeNs / totalTimeNs) * 100, 100) : 0;
  }

  // Calculate Processing percentage for node (100% - IO%)
  getNodeProcessingPercentage(): number {
    if (!this.selectedNode) return 0;
    const ioPercent = this.getNodeIoPercentage();
    return Math.max(100 - ioPercent, 0);
  }

  // Clear node selection
  clearNodeSelection(): void {
    this.selectedNode = null;
    this.selectedNodeTab = 'core';
  }

  // Select node and switch to detail view
  selectNodeForDetail(node: any): void {
    this.selectedNode = node;
    this.selectedNodeTab = 'core';
  }

  // Get Fragment info for selected node
  getNodeFragmentInfo(): any | null {
    if (!this.selectedNode?.fragment_id || !this.analysisData?.fragments) return null;
    return this.analysisData.fragments.find((f: any) => f.id === this.selectedNode.fragment_id);
  }

  // Get Fragment metrics (excluding pipelines and operators)
  getFragmentMetrics(): { key: string; value: any }[] {
    const fragment = this.getNodeFragmentInfo();
    if (!fragment) return [];
    
    const result: { key: string; value: any }[] = [];
    // Add basic fragment info
    if (fragment.backend_addresses?.length) {
      result.push({ key: 'BackendAddresses', value: fragment.backend_addresses.join(', ') });
    }
    if (fragment.instance_ids?.length) {
      result.push({ key: 'InstanceNum', value: fragment.instance_ids.length });
    }
    return result;
  }

  // Get Pipeline info for selected node from fragments
  getNodePipelineFromFragments(): any | null {
    const fragment = this.getNodeFragmentInfo();
    if (!fragment || !this.selectedNode?.pipeline_id) return null;
    return fragment.pipelines?.find((p: any) => p.id === this.selectedNode.pipeline_id);
  }

  // Get Pipeline metrics
  getPipelineMetrics(): { key: string; value: any }[] {
    const pipeline = this.getNodePipelineFromFragments();
    if (!pipeline?.metrics) return [];
    
    return Object.entries(pipeline.metrics)
      .filter(([_, v]) => v !== null && v !== undefined)
      .map(([key, value]) => ({ key, value }));
  }

  // Get operators for selected node from pipeline (for 节点详情 Tab)
  getNodeOperatorsFromPipeline(): any[] {
    const pipeline = this.getNodePipelineFromFragments();
    if (!pipeline?.operators) return [];
    
    const planNodeId = this.selectedNode?.plan_node_id?.toString();
    if (!planNodeId) return pipeline.operators;
    
    // Filter operators by plan_node_id
    return pipeline.operators.filter((op: any) => op.plan_node_id === planNodeId);
  }

  // Get all operators in the pipeline (for full view)
  getAllPipelineOperators(): any[] {
    const pipeline = this.getNodePipelineFromFragments();
    return pipeline?.operators || [];
  }
  
  // Format bytes to human readable
  formatBytes(bytes: any): string {
    if (bytes === null || bytes === undefined) return '0 B';
    const val = Number(bytes);
    if (isNaN(val)) return String(bytes);
    if (val < 1024) return val + ' B';
    if (val < 1024 * 1024) return (val / 1024).toFixed(2) + ' KB';
    if (val < 1024 * 1024 * 1024) return (val / (1024 * 1024)).toFixed(2) + ' MB';
    return (val / (1024 * 1024 * 1024)).toFixed(2) + ' GB';
  }

  // Get diagnostics for a specific node by plan_node_id
  getNodeDiagnostics(node: any): any[] {
    // Note: plan_node_id can be 0, so use explicit null/undefined check
    if (node?.plan_node_id == null || !this.analysisData?.node_diagnostics) {
      return [];
    }
    return this.analysisData.node_diagnostics[node.plan_node_id] || [];
  }

  // Get current diagnostics count based on context (node-specific or aggregated)
  getCurrentDiagnosticsCount(): number {
    if (this.selectedNode) {
      return this.getNodeDiagnostics(this.selectedNode)?.length || 0;
    }
    return this.analysisData?.aggregated_diagnostics?.length || 0;
  }
  
  /**
   * Get total recommendations count (LLM + rule-based diagnostics)
   */
  getTotalRecommendationsCount(): number {
    const llmCount = this.hasLLMAnalysis() ? (this.getRootCauseAnalysis()?.recommendations?.length || 0) : 0;
    const ruleCount = this.analysisData?.aggregated_diagnostics?.length || 0;
    return llmCount + ruleCount;
  }
  
  /**
   * Get effective recommendations count (LLM overrides rules when available)
   */
  getEffectiveRecommendationsCount(): number {
    const llmCount = this.hasLLMAnalysis() ? (this.getRootCauseAnalysis()?.recommendations?.length || 0) : 0;
    // If LLM has recommendations, only count LLM; otherwise count rules
    if (llmCount > 0) {
      return llmCount;
    }
    return this.analysisData?.aggregated_diagnostics?.length || 0;
  }
  
  /**
   * Get merged root cause analysis data
   * Priority: LLM analysis (if available and completed) > Rule-based analysis
   * LLM analysis provides richer natural language descriptions
   */
  getRootCauseAnalysis(): any {
    const llmAnalysis = this.analysisData?.llm_analysis;
    const ruleAnalysis = this.analysisData?.root_cause_analysis;
    
    // If LLM analysis is available and completed, use it
    if (llmAnalysis?.available && llmAnalysis?.status === 'completed') {
      return {
        root_causes: llmAnalysis.root_causes || [],
        causal_chains: llmAnalysis.causal_chains || [],
        summary: llmAnalysis.summary || '',
        recommendations: llmAnalysis.merged_recommendations || [],
        hidden_issues: llmAnalysis.hidden_issues || [],
        source: 'llm'
      };
    }
    
    // Fallback to rule-based analysis
    if (ruleAnalysis) {
      return {
        ...ruleAnalysis,
        source: 'rule'
      };
    }
    
    return null;
  }
  
  /**
   * Check if LLM analysis is available
   */
  hasLLMAnalysis(): boolean {
    const llmAnalysis = this.analysisData?.llm_analysis;
    return llmAnalysis?.available && llmAnalysis?.status === 'completed';
  }
  
  // Get edge path for SVG
  getEdgePath(points: {x: number, y: number}[]): string {
    if (!points || points.length === 0) return '';
    return 'M' + points.map(p => `${p.x},${p.y}`).join(' L');
  }

  // Get edge label position (geometric middle of the path)
  getEdgeLabelPosition(points: {x: number, y: number}[]): {x: number, y: number} {
    if (!points || points.length < 2) return { x: 0, y: 0 };
    
    // Calculate total length
    let totalLength = 0;
    const segments = [];
    for (let i = 0; i < points.length - 1; i++) {
      const dx = points[i+1].x - points[i].x;
      const dy = points[i+1].y - points[i].y;
      const dist = Math.sqrt(dx*dx + dy*dy);
      segments.push({ dist, p1: points[i], p2: points[i+1] });
      totalLength += dist;
    }
    
    let targetLen = totalLength / 2;
    let currentLen = 0;
    
    for (const seg of segments) {
      if (currentLen + seg.dist >= targetLen) {
        // Found the segment
        const remaining = targetLen - currentLen;
        const ratio = remaining / seg.dist;
        return {
          x: seg.p1.x + (seg.p2.x - seg.p1.x) * ratio,
          y: seg.p1.y + (seg.p2.y - seg.p1.y) * ratio,
        };
      }
      currentLen += seg.dist;
    }
    
    // Fallback
    const mid = Math.floor(points.length / 2);
    return { x: points[mid].x, y: points[mid].y };
  }

  getLabelTransform(points: {x: number, y: number}[]): string {
    const pos = this.getEdgeLabelPosition(points);
    return `translate(${pos.x}, ${pos.y})`;
  }

  // Zoom controls - use 0.25 steps for crisper rendering (avoid fractional pixels)
  zoomIn(): void {
    this.zoomLevel = Math.min(Math.round((this.zoomLevel + 0.25) * 4) / 4, 3);
  }

  zoomOut(): void {
    this.zoomLevel = Math.max(Math.round((this.zoomLevel - 0.25) * 4) / 4, 0.25);
  }

  resetZoom(): void {
    this.zoomLevel = 1;
    this.translateX = 0;
    this.translateY = 0;
  }
  
  toggleFullscreen(): void {
    this.isFullscreen = !this.isFullscreen;
    setTimeout(() => {
        // Trigger resize event to re-layout if needed
        window.dispatchEvent(new Event('resize'));
    });
  }

  // Toggle graph direction (BT <-> LR)
  toggleGraphDirection(): void {
    this.graphDirection = this.graphDirection === 'BT' ? 'LR' : 'BT';
    // Rebuild graph with new direction
    if (this.analysisData?.execution_tree) {
      this.buildGraph(this.analysisData.execution_tree);
    }
  }

  // Toggle data flow animation
  toggleFlowAnimation(): void {
    this.isFlowAnimationEnabled = !this.isFlowAnimationEnabled;
  }

  // Export DAG as PNG image
  exportDagAsPng(): void {
    const graphContent = document.querySelector('.graph-content') as HTMLElement;
    if (!graphContent) {
      this.toastrService.warning('Cannot find the graph content', 'Export failed');
      return;
    }

    // Use html2canvas to capture the graph
    import('html2canvas').then(html2canvasModule => {
      const html2canvas = html2canvasModule.default;
      html2canvas(graphContent, {
        backgroundColor: themeColor('--background-basic-color-1', '#ffffff'),
        scale: 2, // Higher resolution
        logging: false,
        useCORS: true,
      }).then(canvas => {
        // Create download link
        const link = document.createElement('a');
        link.download = `profile-dag-${this.currentQueryId || 'export'}.png`;
        link.href = canvas.toDataURL('image/png');
        link.click();
        this.toastrService.success('Graph exported', 'Export succeeded');
      }).catch(err => {
        console.error('Export failed:', err);
        this.toastrService.danger('Failed to export the image', 'Error');
      });
    }).catch(err => {
      console.error('Failed to load html2canvas:', err);
      this.toastrService.danger('Failed to load the export module', 'Error');
    });
  }

  // Panning
  startPan(event: MouseEvent): void {
    // Prevent pan if resizing right panel
    if (this.isResizingRight) return;
    
    // Check if click target is a node or button
    const target = event.target as HTMLElement;
    if (target.closest('.dag-node') || target.closest('button')) return;

    if (event.button === 0) {
      this.isPanning = true;
      this.startX = event.clientX - this.translateX;
      this.startY = event.clientY - this.translateY;
      const viewport = document.querySelector('.dag-center-panel') as HTMLElement; // Use panel cursor
      if (viewport) viewport.style.cursor = 'grabbing';
    }
  }
  
  pan(event: MouseEvent): void {
    if (this.isPanning) {
      event.preventDefault();
      this.translateX = event.clientX - this.startX;
      this.translateY = event.clientY - this.startY;
    }
    // Note: resize handling is now done in startResizeRight() with dedicated listeners
  }
  
  endPan(): void {
    this.isPanning = false;
    this.isResizingRight = false;
    const viewport = document.querySelector('.dag-center-panel') as HTMLElement;
    if (viewport) {
        viewport.style.cursor = 'default';
    }
    document.body.style.cursor = 'default';
  }
  
  // Right Panel Resizing & Toggle
  toggleRightPanel(): void {
    this.isRightPanelCollapsed = !this.isRightPanelCollapsed;
  }
  
  startResizeRight(event: MouseEvent): void {
    event.preventDefault();
    event.stopPropagation();
    this.isResizingRight = true;
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none'; // 防止选择文本
    
    // 记录起始位置和宽度
    const startX = event.clientX;
    const startWidth = this.rightPanelWidth;
    
    // Add global event listeners for reliable resize tracking
    const onMouseMove = (e: MouseEvent) => {
      if (this.isResizingRight) {
        e.preventDefault();
        e.stopPropagation();
        
        // 使用拖动距离来计算新宽度（更可靠）
        const deltaX = startX - e.clientX;
        const newWidth = startWidth + deltaX;
        
        if (newWidth > 200 && newWidth < 800) {
          this.rightPanelWidth = newWidth;
        }
      }
    };
    
    const onMouseUp = (e: MouseEvent) => {
      e.preventDefault();
      this.isResizingRight = false;
      document.body.style.cursor = 'default';
      document.body.style.userSelect = '';
      document.removeEventListener('mousemove', onMouseMove, true);
      document.removeEventListener('mouseup', onMouseUp, true);
    };
    
    // 使用 capture 模式确保优先处理
    document.addEventListener('mousemove', onMouseMove, true);
    document.addEventListener('mouseup', onMouseUp, true);
  }
  
  // Mouse wheel zoom
  onWheel(event: WheelEvent): void {
    event.preventDefault();
    if (event.deltaY < 0) {
      this.zoomIn();
    } else {
      this.zoomOut();
    }
  }
  
  // Get node rank (1-3 for top 3 time-consuming nodes)
  getNodeRank(node: any): number {
    return this.nodeRankMap.get(node.id) || 0;
  }
  
  // Calculate node ranks based on time percentage
  private calculateNodeRanks(): void {
    this.nodeRankMap.clear();
    if (!this.graphNodes || this.graphNodes.length === 0) return;
    
    // Sort nodes by time_percentage descending
    const sorted = [...this.graphNodes]
      .filter(n => n.time_percentage > 0)
      .sort((a, b) => b.time_percentage - a.time_percentage);
    
    // Assign ranks to top 3
    for (let i = 0; i < Math.min(3, sorted.length); i++) {
      this.nodeRankMap.set(sorted[i].id, i + 1);
    }
  }
  
  // Format duration in nanoseconds
  formatDurationNs(ns: any): string {
    if (!ns) return '-';
    const val = Number(ns);
    if (isNaN(val)) return ns;
    
    if (val < 1000) return val + 'ns';
    if (val < 1000000) return (val/1000).toFixed(2) + 'us';
    if (val < 1000000000) return (val/1000000).toFixed(2) + 'ms';
    
    // Convert to seconds
    const totalSeconds = val / 1000000000;
    if (totalSeconds < 60) return totalSeconds.toFixed(2) + 's';
    
    // Convert to human-readable format for larger durations
    const hours = Math.floor(totalSeconds / 3600);
    const minutes = Math.floor((totalSeconds % 3600) / 60);
    const seconds = Math.floor(totalSeconds % 60);
    const millis = Math.round((totalSeconds % 1) * 1000);
    
    let result = '';
    if (hours > 0) result += hours + 'h';
    if (minutes > 0) result += minutes + 'm';
    if (seconds > 0 || (hours === 0 && minutes === 0)) result += seconds + 's';
    if (millis > 0) result += millis + 'ms';
    
    return result || '0s';
  }

  // Fix for Angular base href issue with SVG markers
  get arrowMarkerUrl(): string {
    return `url(${this.location.path()}#dag-arrow)`;
  }

  private normalizedOperatorName(node: any): string {
    return (node?.operator_name || '').toUpperCase();
  }

  isScanNode(node: any): boolean {
    return this.normalizedOperatorName(node).includes('SCAN');
  }

  isJoinNode(node: any): boolean {
    return this.normalizedOperatorName(node).includes('JOIN');
  }

  isProjectNode(node: any): boolean {
    return this.normalizedOperatorName(node).includes('PROJECT');
  }

  isExchangeNode(node: any): boolean {
    return this.normalizedOperatorName(node).includes('EXCHANGE');
  }

  isAggregationNode(node: any): boolean {
    const name = this.normalizedOperatorName(node);
    return name.includes('AGGREGATION') || name.includes('AGG');
  }

  // Get node header class based on operator type (Figure 1 style)
  getNodeHeaderClass(node: any): string {
    if (this.getNodeRank(node) === 1) {
      return 'header-red';
    }
    if (this.isScanNode(node) || this.isJoinNode(node)) {
      return 'header-orange';
    }
    return 'header-gray';
  }

  // Get progress bar color based on node rank and type
  // Top 1: Red (most time consuming)
  // Top 2/3 or SCAN/JOIN: Orange
  // Others: Gray
  getProgressColor(node: any): string {
    const rank = this.getNodeRank(node);
    if (rank === 1) return themeColor('--color-danger-default', '#ff3d71');
    if (rank > 0 || this.isScanNode(node) || this.isJoinNode(node)) return themeColor('--color-warning-default', '#ffaa00');
    return themeColor('--border-basic-color-4', '#c5cee0');
  }

  // Toggle functions for right panel sections
  toggleTop10(): void { this.isTop10Collapsed = !this.isTop10Collapsed; }
  toggleSummary(): void { this.isSummaryCollapsed = !this.isSummaryCollapsed; }
  toggleDiagnosis(): void { this.isDiagnosisCollapsed = !this.isDiagnosisCollapsed; }
  toggleMemoryView(): void { 
    this.showMemoryView = !this.showMemoryView; 
  }

  // Copy profile content to clipboard
  copyProfileToClipboard(): void {
    if (!this.currentProfileDetail) {
      return;
    }

    // Use Clipboard API
    if (navigator.clipboard && window.isSecureContext) {
      navigator.clipboard.writeText(this.currentProfileDetail)
        .then(() => {
          this.toastrService.success('Profile content copied to clipboard', 'Copy succeeded');
        })
        .catch(err => {
          console.error('Failed to copy:', err);
          this.fallbackCopy();
        });
    } else {
      // Fallback for older browsers or non-secure contexts
      this.fallbackCopy();
    }
  }

  // Fallback copy method for older browsers
  private fallbackCopy(): void {
    const textArea = document.createElement('textarea');
    textArea.value = this.currentProfileDetail;
    textArea.style.position = 'fixed';
    textArea.style.left = '-999999px';
    document.body.appendChild(textArea);
    textArea.focus();
    textArea.select();

    try {
      const successful = document.execCommand('copy');
      if (successful) {
        this.toastrService.success('Profile content copied to clipboard', 'Copy succeeded');
      } else {
        this.toastrService.warning('Copy failed, please copy manually', 'Hint');
      }
    } catch (err) {
      console.error('Failed to copy:', err);
      this.toastrService.warning('Copy failed, please copy manually', 'Hint');
    } finally {
      document.body.removeChild(textArea);
    }
  }

  // Copy text to clipboard (for parameter commands)
  copyToClipboard(text: string): void {
    if (!text) return;
    
    if (navigator.clipboard && window.isSecureContext) {
      navigator.clipboard.writeText(text)
        .then(() => {
          this.toastrService.success('Copied to clipboard', 'Copy succeeded');
        })
        .catch(err => {
          console.error('Failed to copy:', err);
          this.toastrService.warning('Copy failed', 'Hint');
        });
    } else {
      // Fallback
      const textArea = document.createElement('textarea');
      textArea.value = text;
      textArea.style.position = 'fixed';
      textArea.style.left = '-999999px';
      document.body.appendChild(textArea);
      textArea.select();
      try {
        document.execCommand('copy');
        this.toastrService.success('Copied to clipboard', 'Copy succeeded');
      } catch (err) {
        this.toastrService.warning('Copy failed', 'Hint');
      } finally {
        document.body.removeChild(textArea);
      }
    }
  }
}
