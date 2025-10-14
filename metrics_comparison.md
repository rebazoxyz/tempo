# Tempo Benchmark Comparison

Main commit: ``/Users/yongkang/Code/tempo/tempo/debug_main.log``
Feature commit: ``/Users/yongkang/Code/tempo/tempo/debug_feature.log``

| Metric | Statistic | Main | Feature | Abs Diff | % Change |
| --- | --- | --- | --- | --- | --- |
| Build Payload Time | Average | 358.378 ms | 318.035 ms | -40.343 ms | -11.3% |
|  | Median | 321.440 ms | 318.483 ms | -2.957 ms | -0.9% |
|  | Min | 252.534 ms | 240.773 ms | -11.761 ms | -4.7% |
|  | Max | 578.119 ms | 404.723 ms | -173.396 ms | -30.0% |
|  | Std Dev | 103.123 ms | 48.856 ms | -54.267 ms | -52.6% |
| State Root Computation | Average | 6.872 ms | 4.594 ms | -2.278 ms | -33.1% |
|  | Median | 4.785 ms | 4.399 ms | -0.386 ms | -8.1% |
|  | Min | 2.892 ms | 2.940 ms | +0.048 ms | +1.7% |
|  | Max | 25.717 ms | 6.628 ms | -19.089 ms | -74.2% |
|  | Std Dev | 7.115 ms | 1.115 ms | -6.000 ms | -84.3% |
| Explicit State Root Task | Average | 9.871 ms | 8.496 ms | -1.375 ms | -13.9% |
|  | Median | 5.047 ms | 4.473 ms | -0.574 ms | -11.4% |
|  | Min | 0.004 ms | 0.004 ms | +0.000 ms | +5.7% |
|  | Max | 83.490 ms | 63.391 ms | -20.098 ms | -24.1% |
|  | Std Dev | 13.417 ms | 9.773 ms | -3.644 ms | -27.2% |
| Block Added to Canonical Chain | Average | 2122.482 ms | 2003.474 ms | -119.008 ms | -5.6% |
|  | Median | 1902.425 ms | 1963.036 ms | +60.611 ms | +3.2% |
|  | Min | 1340.895 ms | 1277.200 ms | -63.695 ms | -4.8% |
|  | Max | 3388.789 ms | 2865.275 ms | -523.514 ms | -15.4% |
|  | Std Dev | 728.569 ms | 513.838 ms | -214.731 ms | -29.5% |
