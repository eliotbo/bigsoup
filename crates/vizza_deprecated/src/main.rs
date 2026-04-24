use anyhow::Result;
use vizza::PlotBuilder;

fn main() -> Result<()> {
    // Use the PlotBuilder API to create and run the plot
    PlotBuilder::new().run()
}
